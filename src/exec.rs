use anyhow::Context;
use std::collections::HashSet;
use std::env::VarError;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::{fs, io};

use crate::redoxfs::{archive_image, extract_tar, run_install_mount, run_install_to_dir, RedoxFs};
use crate::{host_target, redoxer_dir, status_error, target};

// extra disk space to fit large projects
const DISK_SIZE: u64 = 3 * 1024 * 1024 * 1024;
// need to fit under the default RAM
const DISK_SIZE_LIVE: u64 = 1 * 1024 * 1024 * 1024;

pub fn qemu_executable() -> &'static str {
    match target() {
        "x86_64-unknown-redox" => "qemu-system-x86_64",
        "aarch64-unknown-redox" => "qemu-system-aarch64",
        "i586-unknown-redox" | "i686-unknown-redox" => "qemu-system-i386",
        "riscv64gc-unknown-redox" => "qemu-system-riscv64",
        _ => panic!("Unknown target architecture for QEMU"),
    }
}

pub fn qemu_has_kvm() -> bool {
    fn get_arch(triple: &str) -> &str {
        triple.split('-').next().unwrap_or(triple)
    }
    let host = get_arch(host_target());
    let target = get_arch(target());
    Path::new("/dev/kvm").exists()
        && match (host, target) {
            ("x86_64", "x86_64" | "i586" | "i686") => true,
            // https://gitlab.redox-os.org/redox-os/redox/-/issues/1714
            ("aarch64", "aarch64") => false,
            (_, _) => false,
        }
}

pub fn qemu_use_uefi() -> bool {
    match target() {
        "x86_64-unknown-redox" => false,
        "i586-unknown-redox" | "i686-unknown-redox" => false,
        "aarch64-unknown-redox" => true,
        "riscv64gc-unknown-redox" => true,
        _ => panic!("Unknown target architecture for QEMU"),
    }
}

pub fn qemu_use_live_disk() -> bool {
    match target() {
        "x86_64-unknown-redox" => false,
        "i586-unknown-redox" | "i686-unknown-redox" => false,
        "aarch64-unknown-redox" => true,
        "riscv64gc-unknown-redox" => true,
        _ => panic!("Unknown target architecture for QEMU"),
    }
}

pub fn qemu_disk_size() -> u64 {
    if qemu_use_live_disk() {
        DISK_SIZE_LIVE
    } else {
        DISK_SIZE
    }
}

pub fn qemu_default_args() -> Vec<&'static str> {
    #[rustfmt::skip]
    let mut default_args = vec![
        "-cpu", "max",
        "-m", "2048",
        "-smp", "4",
        "-netdev", "user,id=net0",
        "-device", "e1000,netdev=net0",
    ];
    default_args.extend(match target() {
        #[rustfmt::skip]
        "i586-unknown-redox" | "i686-unknown-redox" | "x86_64-unknown-redox" => vec![
            "-machine", "q35", 
            "-serial", "mon:stdio",
            "-device", "isa-debugcon,chardev=log",
            "-device", "isa-debug-exit",
        ],
        #[rustfmt::skip]
        "aarch64-unknown-redox" => {
            let (bios_arg, bios_file) = if Path::new("/usr/share/AAVMF/AAVMF_CODE.fd").exists() {
                ("-bios", "/usr/share/AAVMF/AAVMF_CODE.fd")
            } else if Path::new("/usr/share/qemu/edk2-aarch64-code.fd").exists() {
                ("-drive", "if=pflash,format=raw,unit=0,file=/usr/share/qemu/edk2-aarch64-code.fd,readonly=on")
            } else {
                todo!("Can't figure out where is the BIOS file!")
            };
            vec![
                "-machine", "virt",
                "-serial", "chardev:debug",
                "-mon", "chardev=debug",
                bios_arg, bios_file,
                "-chardev", "stdio,id=debug,signal=off,mux=on",
                "-semihosting-config", "enable=on,target=native,userspace=on"
            ]
        }
        #[rustfmt::skip]
        "riscv64gc-unknown-redox" => vec![
            "-machine", "virt",
            // TODO: Add more devices
            "-semihosting-config", "enable=on,target=native,userspace=on"
        ],
        _ => panic!("Unknown target architecture for QEMU"),
    });
    default_args
}

static BASE_TOML: &'static str = include_str!("../res/base.toml");
static GUI_TOML: &'static str = include_str!("../res/gui.toml");

fn bootloader() -> anyhow::Result<PathBuf> {
    let bootloader_bin = redoxer_dir().join("bootloader.bin");
    if !bootloader_bin.is_file() {
        eprintln!("redoxer: building bootloader");

        let bootloader_dir = redoxer_dir().join("bootloader");
        if bootloader_dir.is_dir() {
            fs::remove_dir_all(&bootloader_dir)?;
        }
        fs::create_dir_all(&bootloader_dir)?;

        let mut config = redox_installer::Config::default();
        config.files.push(redox_installer::FileConfig {
            path: "/etc/pkg.d/50_redox".to_string(),
            data: "https://static.redox-os.org/pkg".to_string(),
            ..Default::default()
        });
        config
            .packages
            .insert("bootloader".to_string(), Default::default());
        redox_installer::install(config, &bootloader_dir)
            .map_err(|err| io::Error::new(io::ErrorKind::Other, format!("{}", err)))?;

        fs::rename(
            &bootloader_dir.join(if qemu_use_uefi() {
                "boot/bootloader-live.efi"
            } else {
                "boot/bootloader.bios"
            }),
            &bootloader_bin,
        )?;
    }
    Ok(bootloader_bin)
}

fn base(bootloader_bin: &Path, gui: bool, fuse: bool) -> anyhow::Result<PathBuf> {
    let name = if gui { "gui" } else { "base" };
    let ext = if fuse { "bin" } else { "tar" };

    let tar_file = redoxer_dir().join(format!("{}.{}", name, ext));
    let base_tar = redoxer_dir().join(format!("{}.{}", name, "tar"));
    if !tar_file.is_file() {
        eprintln!("redoxer: building {}", name);

        let base_dir = redoxer_dir().join(name);
        if base_dir.is_dir() {
            fs::remove_dir_all(&base_dir)?;
        }
        fs::create_dir_all(&base_dir)?;

        let base_partial = redoxer_dir().join(format!("{}.{}.partial", name, ext));
        if base_partial.is_file() {
            fs::remove_file(&base_partial)?;
        }

        let mut config: redox_installer::Config =
            toml::from_str(if gui { GUI_TOML } else { BASE_TOML })
                .map_err(|err| io::Error::new(io::ErrorKind::Other, format!("{}", err)))?;
        config.general.live_disk = Some(qemu_use_live_disk());

        if fuse {
            run_install_mount(
                config,
                bootloader_bin,
                qemu_use_uefi(),
                qemu_disk_size(),
                &base_tar,
                &base_dir,
                &base_partial,
            )?;
        } else {
            run_install_to_dir(config, &base_dir)?;

            eprintln!("redoxer: compressing {}", name);
            Command::new("tar")
                .arg("-c")
                .arg("-p")
                .arg("-f")
                .arg(&base_partial)
                .arg("-C")
                .arg(&base_dir)
                .arg(".")
                .status()
                .and_then(status_error)?;
        }

        fs::rename(&base_partial, &tar_file)?;
        // fs::remove_dir_all(&base_dir)?;
    }
    Ok(tar_file)
}

struct RedoxerConfig {
    qemu_binary: Option<String>,
    qemu_args: Option<String>,
    fuse: Option<bool>,
    // TODO: gui: bool, or generalize it into any config TOML
}

fn apply_qemu_args(cmd: &mut Command, default: Vec<&str>, args_opt: Option<Vec<&str>>) {
    let final_args = if let Some(user_args) = args_opt {
        let user_opts: HashSet<&str> = user_args
            .iter()
            .filter(|arg| arg.starts_with('-'))
            .copied()
            .collect();

        let mut merged_args: Vec<String> = Vec::new();
        let mut i = 0;
        while i < default.len() {
            let opt = &default[i];
            if !opt.starts_with('-') {
                continue; // shouldn't go here
            }

            let is_single_flag = default.get(i + 1).map_or(true, |a| a.starts_with('-'));

            if !user_opts.contains(opt) {
                merged_args.push(opt.to_string());
                if !is_single_flag {
                    merged_args.push(default[i + 1].to_string());
                }
            }

            i += if is_single_flag { 1 } else { 2 };
        }

        merged_args.extend(user_args.into_iter().map(String::from));
        merged_args
    } else {
        default.into_iter().map(String::from).collect()
    };

    cmd.args(final_args);
}

fn installed(program: &str) -> io::Result<bool> {
    process::Command::new("which")
        .arg(program)
        .stdout(process::Stdio::null())
        .status()
        .map(|x| x.success())
}

fn inner(
    arguments: &[String],
    config: &RedoxerConfig,
    folder_opt: Option<String>,
    gui: bool,
    output_opt: Option<String>,
) -> anyhow::Result<i32> {
    let qemu_binary = config.qemu_binary.as_deref().unwrap_or(qemu_executable());

    if !installed(qemu_binary)? {
        eprintln!(
            "redoxer: {} not found, please install before continuing",
            qemu_executable()
        );
        process::exit(1);
    }
    let kvm = qemu_has_kvm();

    let fuse = config
        .fuse
        .unwrap_or_else(|| Path::new("/dev/fuse").exists());

    if fuse {
        if !installed("fusermount")? {
            eprintln!("redoxer: fuse not found, please install before continuing");
            process::exit(1);
        }
    } else if !installed("tar")? {
        eprintln!("redoxer: tar not found, please install before continuing");
        process::exit(1);
    }

    let bootloader_bin = bootloader().context("unable to init bootloader")?;
    let tar_file = base(&bootloader_bin, gui, fuse).context("unable to init base")?;

    let tempdir = tempfile::tempdir().context("unable to create tempdir")?;

    let code = {
        let redoxer_bin = tempdir.path().join("redoxer.bin");
        if fuse {
            Command::new("cp")
                .arg(&tar_file)
                .arg(&redoxer_bin)
                .status()
                .and_then(status_error)
                .context("copy base to redoxer bin failed")?;
        }

        let dest_dir = tempdir.path().join("redoxer");
        fs::create_dir_all(&dest_dir).context("unable to create redoxer dir")?;

        {
            let redoxfs_opt = if fuse {
                Some(RedoxFs::new(&redoxer_bin, &dest_dir).context("unable to init redoxfs")?)
            } else {
                extract_tar(&tar_file, &dest_dir)?;
                None
            };

            let mut redoxerd_config = String::new();
            for arg in arguments.iter() {
                // Replace absolute path to folder with /root in command name
                // TODO: make this activated by a flag
                if let Some(ref folder) = folder_opt {
                    let folder_canonical_path =
                        fs::canonicalize(&folder).context("unable to canonalize")?;
                    let folder_canonical = folder_canonical_path.to_str().ok_or(io::Error::new(
                        io::ErrorKind::Other,
                        "folder path is not valid UTF-8",
                    ))?;
                    if arg.starts_with(&folder_canonical) {
                        let arg_replace = arg.replace(folder_canonical, "/root");
                        eprintln!(
                            "redoxer: replacing '{}' with '{}' in arguments",
                            arg, arg_replace
                        );
                        redoxerd_config.push_str(&arg_replace);
                        redoxerd_config.push('\n');
                        continue;
                    }
                }

                redoxerd_config.push_str(&arg);
                redoxerd_config.push('\n');
            }
            fs::write(dest_dir.join("etc/redoxerd"), redoxerd_config)
                .context("unable to write redoxerd config")?;

            if let Some(ref folder) = folder_opt {
                eprintln!("redoxer: copying '{}' to '/root'", folder);

                let root_dir = dest_dir.join("root");
                Command::new("rsync")
                    .arg("--archive")
                    .arg(&folder)
                    .arg(&root_dir)
                    .status()
                    .and_then(status_error)
                    .context("rsync failed")?;
            }

            if let Some(mut redoxfs) = redoxfs_opt {
                redoxfs.unmount().context("unable to unmount")?;
            }
        }

        if !fuse {
            archive_image(
                &redoxer_bin,
                &dest_dir,
                &bootloader_bin,
                qemu_use_uefi(),
                qemu_disk_size(),
            )?;
        }

        let redoxer_log = tempdir.path().join("redoxer.log");
        let mut command = Command::new(qemu_binary);

        let chardev = format!("file,id=log,path={}", redoxer_log.display());
        let drive = format!("file={},format=raw,if=virtio", redoxer_bin.display());
        let mut default_args = qemu_default_args();
        default_args.extend(vec!["-chardev", &chardev, "-drive", &drive]);
        if kvm {
            default_args.push("-accel");
            default_args.push("kvm");
        }
        if !gui {
            default_args.push("-nographic");
            default_args.push("-vga");
            default_args.push("none");
        }

        apply_qemu_args(
            &mut command,
            default_args,
            config.qemu_args.as_ref().map(|s| s.split(" ").collect()),
        );

        let status = command.status().context("unable to get redoxer status")?;

        eprintln!();

        let code = match status.code() {
            Some(51) => {
                eprintln!("## redoxer (success) ##");
                0
            }
            Some(53) => {
                eprintln!("## redoxer (failure) ##");
                1
            }
            _ => {
                eprintln!("## redoxer (failure, qemu exit status {:?} ##", status);
                2
            }
        };

        if let Some(output) = output_opt {
            fs::copy(&redoxer_log, output)?;
        } else {
            print!("{}", fs::read_to_string(&redoxer_log)?);
        }

        code
    };

    tempdir.close()?;

    Ok(code)
}

fn usage() {
    eprintln!("redoxer exec [-f|--folder folder] [-g|--gui] [-h|--help] [-o|--output file] [--] <command> [arguments]...");
    process::exit(1);
}

pub fn main(args: &[String]) {
    // Matching flags
    let mut matching = true;
    // Folder to copy
    let mut folder_opt = None;
    // Run with GUI
    let mut gui = false;
    // File to put command output into
    let mut output_opt = None;
    // Arguments to pass to command
    let mut arguments = Vec::new();

    let mut args = args.iter().cloned().skip(2);
    while let Some(arg) = args.next() {
        match (arg.as_str(), matching) {
            ("-f" | "--folder", true) => match args.next() {
                Some(folder) => folder_opt = Some(folder),
                None => usage(),
            },
            ("-g" | "--gui", true) => gui = true,
            // TODO: argument for replacing the folder path with /root when found in arguments
            ("-h" | "--help", true) => usage(),
            ("-o" | "--output", true) => match args.next() {
                Some(output) => output_opt = Some(output),
                None => usage(),
            },
            // TODO: "-p" | "--package"
            ("--", true) => matching = false,
            _ => {
                matching = false;
                arguments.push(arg);
            }
        }
    }

    if arguments.is_empty() {
        usage();
    }

    if folder_opt.is_none() {
        if let Some(cmd) = arguments.get(0) {
            if Path::new(cmd).is_file() {
                if !cmd.contains('/') {
                    eprintln!(
                        "WARN: Skipping copy, you might mean to run exec with ./{}",
                        cmd
                    )
                } else {
                    folder_opt = Some(cmd.to_string());
                }
            }
        }
    }

    use std::env::var;
    fn parse_bool_env(name: &str) -> Option<bool> {
        match var(name).as_deref() {
            Ok("true" | "1") => Some(true),
            Ok("false" | "0") => Some(false),
            Ok(arg) => panic!("invalid argument {} for REDOXER_USE_FUSE", arg),
            Err(VarError::NotPresent) => None,
            Err(VarError::NotUnicode(_)) => panic!("non-utf8 argument for {}", name),
        }
    }
    let config = RedoxerConfig {
        qemu_binary: var("REDOXER_QEMU_BINARY").ok(),
        qemu_args: var("REDOXER_QEMU_ARGS").ok(),
        fuse: parse_bool_env("REDOXER_USE_FUSE"),
    };

    match inner(&arguments, &config, folder_opt, gui, output_opt) {
        Ok(code) => {
            process::exit(code);
        }
        Err(err) => {
            eprintln!("redoxer exec: {:#}", err);
            process::exit(3);
        }
    }
}
