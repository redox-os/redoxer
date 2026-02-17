use anyhow::{bail, Context};
use std::collections::{HashMap, HashSet};
use std::env::VarError;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::{fs, io};

use crate::redoxfs::{
    archive_image, expand_disk, extract_tar, run_install_mount, run_install_to_dir, shrink_disk,
    RedoxFs,
};
use crate::writer::write_redoxerd_config;
use crate::{host_target, redoxer_dir, status_error, target};

// extra disk space to fit large projects
const DISK_SIZE: u64 = 3 * 1024 * 1024 * 1024;
// need to fit under the default RAM
const DISK_SIZE_LIVE: u64 = 1024 * 1024 * 1024;

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

static BASE_TOML: &str = include_str!("../res/base.toml");
static GUI_TOML: &str = include_str!("../res/gui.toml");

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
            .map_err(|err| io::Error::other(format!("{}", err)))?;

        fs::rename(
            bootloader_dir.join(if qemu_use_uefi() {
                "boot/bootloader-live.efi"
            } else {
                "boot/bootloader.bios"
            }),
            &bootloader_bin,
        )?;
    }
    Ok(bootloader_bin)
}

/// creating a base image, returns the base image, bool if orbital exists
fn base(
    bootloader_bin: &Path,
    name: &str,
    config_str: &str,
    fuse: bool,
) -> anyhow::Result<(PathBuf, bool)> {
    let ext = if fuse { "bin" } else { "tar" };

    let base_file = redoxer_dir().join(format!("{}.{}", name, ext));
    let base_tar = redoxer_dir().join(format!("{}.{}", name, "tar"));
    let base_toml = redoxer_dir().join(format!("{}.{}", name, "toml"));

    let mut config: redox_installer::Config =
        toml::from_str(config_str).context("Unable to parse install-config")?;
    let has_orbital = config.packages.contains_key("orbital");

    if base_toml.is_file() && base_file.is_file() {
        let r = fs::read_to_string(&base_toml).context("Unable to read base toml")?;
        if r != config_str {
            eprintln!("redoxer: clearing old {}", name);
            fs::remove_file(&base_toml).context("Unable to delete base toml")?;
            fs::remove_file(&base_file).context("Unable to delete base bin/tar")?;
        }
    }
    if !base_file.is_file() {
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

            // only shrink disk outside CI
            if !base_tar.exists() {
                eprintln!("redoxer: shrinking {}", name);
                shrink_disk(&base_partial)?;
            }
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

        fs::rename(&base_partial, &base_file)?;
        if base_dir.is_dir() {
            fs::remove_dir_all(&base_dir)?;
        }
        fs::write(base_toml, config_str)?;
    }
    Ok((base_file, has_orbital))
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

            let is_single_flag = default.get(i + 1).is_none_or(|a| a.starts_with('-'));

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

fn inner(config: &RedoxerExecConfig) -> anyhow::Result<i32> {
    let qemu_binary = config.qemu_binary.as_deref().unwrap_or(qemu_executable());

    if !installed(qemu_binary)? {
        eprintln!(
            "redoxer: {} not found, please install before continuing",
            qemu_binary
        );
        process::exit(1);
    }
    let kvm = qemu_has_kvm();

    let fuse = config.fuse;

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
    let (base_file, gui) = base(
        &bootloader_bin,
        &config.config_name,
        &config.config_toml,
        fuse,
    )
    .context("unable to init base")?;

    eprintln!("redoxer: creating temporary disk");
    let tempdir = tempfile::tempdir().context("unable to create tempdir")?;
    let redoxer_bin = tempdir.path().join("redoxer.bin");
    let dest_dir = tempdir.path().join("redoxer");

    let code = {
        if fuse {
            Command::new("cp")
                .arg(&base_file)
                .arg(&redoxer_bin)
                .status()
                .and_then(status_error)
                .context("copy base to redoxer bin failed")?;

            expand_disk(&redoxer_bin, qemu_disk_size())?;
        }

        fs::create_dir_all(&dest_dir).context("unable to create redoxer dir")?;

        {
            let redoxfs_opt = if fuse {
                Some(RedoxFs::new(&redoxer_bin, &dest_dir).context("unable to init redoxfs")?)
            } else {
                extract_tar(&base_file, &dest_dir)?;
                None
            };

            write_redoxerd_config(
                &dest_dir,
                &config.arguments,
                config.folders.get("root").map(|s| s.as_str()),
            )?;

            for (sysroot, folder) in config.folders.iter() {
                eprintln!("redoxer: copying '{folder}' to '/{sysroot}'",);

                let dst_dir = dest_dir.join(sysroot);
                if !dst_dir.is_dir() {
                    fs::create_dir_all(&dst_dir)
                        .context("unable to create destination directory")?;
                }
                Command::new("rsync")
                    .arg("--archive")
                    .arg(folder)
                    .arg(&dst_dir)
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

        if let Some(output) = &config.output {
            fs::copy(&redoxer_log, output)?;
        } else {
            print!("{}", fs::read_to_string(&redoxer_log)?);
        }

        code
    };

    if code == 0 && !config.artifacts.is_empty() {
        let redoxfs_opt = if fuse {
            Some(RedoxFs::new(&redoxer_bin, &dest_dir).context("unable to init redoxfs")?)
        } else {
            unimplemented!()
        };

        for (sysroot, folder) in config.artifacts.iter() {
            eprintln!("redoxer: copying '/{sysroot}' to '{folder}'");

            let dst_dir = Path::new(folder);
            if !dst_dir.is_dir() {
                fs::create_dir_all(dst_dir).context("unable to create destination directory")?;
            }
            Command::new("rsync")
                .arg("--archive")
                .arg(format!("{}/", dest_dir.join(sysroot).display()))
                .arg(dst_dir)
                .status()
                .and_then(status_error)
                .context("rsync failed")?;
        }

        if let Some(mut redoxfs) = redoxfs_opt {
            redoxfs.unmount().context("unable to unmount")?;
        }
    }

    tempdir.close()?;

    Ok(code)
}

fn usage() -> ! {
    eprintln!("redoxer exec [-f|--folder folder] [-f|--folder folder:/path/in/redox] [-a|--artifact folder] [-a|--artifact folder:/path/in/redox] [-g|--gui] [-h|--help] [-i|--install-config] [-o|--output file] [--] <command> [arguments]...");
    process::exit(1);
}

#[derive(Clone, Default)]
pub struct RedoxerExecConfig {
    // Qemu config
    pub qemu_binary: Option<String>,
    pub qemu_args: Option<String>,
    pub fuse: bool,
    // Installer config
    pub config_name: String,
    pub config_toml: String,
    // Folders to copy (host -> qemu)
    pub folders: HashMap<String, String>,
    // Folders to extract (qemu -> host)
    pub artifacts: HashMap<String, String>,
    // Output log
    pub output: Option<String>,
    // Commands to execute
    pub arguments: Vec<String>,
}

impl RedoxerExecConfig {
    pub fn new(mut args: impl Iterator<Item = String>) -> anyhow::Result<Self> {
        use std::env::var;
        fn parse_bool_env(name: &str) -> Option<bool> {
            match var(name).as_deref() {
                Ok("true" | "1") => Some(true),
                Ok("false" | "0") => Some(false),
                Ok(arg) => panic!("invalid argument {} for {}", arg, name),
                Err(VarError::NotPresent) => None,
                Err(VarError::NotUnicode(_)) => panic!("non-utf8 argument for {}", name),
            }
        }

        fn parse_folder(
            map: &mut HashMap<String, String>,
            folder: String,
            argname: &str,
        ) -> anyhow::Result<()> {
            let (dir, sysroot): (String, String) = match folder
                .chars()
                .filter(|c| *c == ':')
                .count()
            {
                0 => (folder, "/root".to_string()),
                1 => {
                    let mut split = folder.split(":");
                    let r = (
                        split.next().unwrap().to_string(),
                        split.next().unwrap().to_string(),
                    );
                    if !r.1.starts_with("/") {
                        bail!("path on {argname} with format 'directory:path' must be an absolute path (starting with '/')");
                    }
                    r
                }
                _ => bail!("{argname} can be 'directory' or 'directory:path'"),
            };
            if map.insert(sysroot[1..].to_string(), dir).is_some() {
                bail!("path on {argname} with format 'directory:path' must be unique");
            }

            Ok(())
        }

        let mut config = RedoxerExecConfig {
            qemu_binary: var("REDOXER_QEMU_BINARY").ok(),
            qemu_args: var("REDOXER_QEMU_ARGS").ok(),
            fuse: parse_bool_env("REDOXER_USE_FUSE")
                .unwrap_or_else(|| Path::new("/dev/fuse").exists()),
            config_name: "base".into(),
            config_toml: BASE_TOML.into(),
            // other options should be passed from args
            ..Default::default()
        };

        // Matching flags
        let mut matching = true;
        while let Some(arg) = args.next() {
            match (arg.as_str(), matching) {
                ("-f" | "--folder", true) => match args.next() {
                    Some(folder) => parse_folder(&mut config.folders, folder, "--folder")?,
                    None => bail!("--folder requires a path to a directory"),
                },
                ("-a" | "--artifact", true) => match args.next() {
                    Some(folder) => parse_folder(&mut config.artifacts, folder, "--artifact")?,
                    None => bail!("--folder requires a path to a directory"),
                },
                ("-g" | "--gui", true) => {
                    config.config_name = "gui".into();
                    config.config_toml = GUI_TOML.into();
                }
                ("-i" | "--install-config", true) => match args.next() {
                    Some(file) => {
                        let path = Path::new(&file);
                        config.config_name =
                            path.file_stem().unwrap().to_string_lossy().to_string();
                        if &config.config_name == "base" || &config.config_name == "gui" {
                            bail!("--install-config file path cannot be 'base' or 'gui'");
                        }
                        config.config_toml =
                            fs::read_to_string(path).expect("unable to read --install-config file");
                    }
                    None => bail!("--output requires a path to a directory"),
                },
                ("-h" | "--help", true) => bail!(""),
                ("-o" | "--output", true) => match args.next() {
                    Some(output) => config.output = Some(output),
                    None => bail!("--output requires a path to a directory"),
                },
                ("--", true) => matching = false,
                _ => {
                    matching = false;
                    config.arguments.push(arg);
                }
            }
        }

        if !config.folders.contains_key("root") {
            if let Some(cmd) = config.arguments.first() {
                if Path::new(cmd).is_file() {
                    if !cmd.contains('/') {
                        eprintln!(
                            "WARN: Skipping copy, you might mean to run exec with ./{}",
                            cmd
                        )
                    } else {
                        config.folders.insert("root".to_string(), cmd.to_string());
                    }
                }
            }
        }

        if !config.artifacts.is_empty() && !config.fuse {
            bail!("--artifact requires REDOXER_USE_FUSE=true");
        }

        Ok(config)
    }

    pub fn to_args(&self) -> Vec<String> {
        let mut args = Vec::new();

        for (sysroot, host_dir) in &self.folders {
            args.push("--folder".to_string());
            args.push(format!("{}:/{}", host_dir, sysroot));
        }

        for (sysroot, host_dir) in &self.artifacts {
            args.push("--artifact".to_string());
            args.push(format!("{}:/{}", host_dir, sysroot));
        }

        if self.config_name == "gui" {
            args.push("--gui".to_string());
        }

        if let Some(ref output) = self.output {
            args.push("--output".to_string());
            args.push(output.clone());
        }

        if !self.arguments.is_empty() {
            args.push("--".to_string());

            for arg in &self.arguments {
                args.push(arg.clone());
            }
        }

        args
    }
}

pub fn main(args: &[String]) {
    let config = match RedoxerExecConfig::new(args.iter().cloned().skip(2)) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("{:?}", err);
            usage();
        }
    };

    if config.arguments.is_empty() {
        usage();
    }
    match inner(&config) {
        Ok(code) => {
            process::exit(code);
        }
        Err(err) => {
            eprintln!("redoxer exec: {:#}", err);
            process::exit(3);
        }
    }
}
