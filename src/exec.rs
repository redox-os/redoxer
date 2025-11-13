use anyhow::Context;
use redoxfs::{archive_at, DiskSparse, FileSystem, TreePtr, BLOCK_SIZE};
use std::collections::HashSet;
use std::env::VarError;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{fs, io};

use crate::redoxfs::RedoxFs;
use crate::{installed, redoxer_dir, status_error, syscall_error};

const BOOTLOADER_SIZE: usize = 2 * 1024 * 1024;
const DISK_SIZE: u64 = 3 * 1024 * 1024 * 1024;

static BASE_TOML: &'static str = include_str!("../res/base.toml");
static GUI_TOML: &'static str = include_str!("../res/gui.toml");

/// Redoxer is used for testing out apps in redox OS environment.
/// For this reason no live image is required
const INSTALL_LIVE_IMAGE: bool = false;

fn bootloader() -> io::Result<PathBuf> {
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
        let cookbook: Option<&str> = None;
        redox_installer::install(config, &bootloader_dir, cookbook, INSTALL_LIVE_IMAGE, None)
            .map_err(|err| io::Error::new(io::ErrorKind::Other, format!("{}", err)))?;

        fs::rename(
            &bootloader_dir.join("boot/bootloader.bios"),
            &bootloader_bin,
        )?;
    }
    Ok(bootloader_bin)
}

fn base(bootloader_bin: &Path, gui: bool, fuse: bool) -> io::Result<PathBuf> {
    let name = if gui { "gui" } else { "base" };
    let ext = if fuse { "bin" } else { "tar" };

    let base_bin = redoxer_dir().join(format!("{}.{}", name, ext));
    let base_tar = redoxer_dir().join(format!("{}.{}", name, "tar"));
    if !base_bin.is_file() {
        eprintln!("redoxer: building {}", name);

        let base_dir = redoxer_dir().join(name);
        if base_dir.is_dir() {
            fs::remove_dir_all(&base_dir)?;
        }
        fs::create_dir_all(&base_dir)?;

        let base_partial = redoxer_dir().join(format!("{}.{}.partial", name, ext));

        if fuse {
            let disk = DiskSparse::create(&base_partial, DISK_SIZE).map_err(syscall_error)?;

            let bootloader = {
                let mut bootloader = fs::read(bootloader_bin)?.to_vec();

                // Pad bootloader to 2 MiB
                while bootloader.len() < BOOTLOADER_SIZE {
                    bootloader.push(0);
                }

                bootloader
            };

            let ctime = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
            let fs = FileSystem::create_reserved(
                disk,
                None,
                &bootloader,
                ctime.as_secs(),
                ctime.subsec_nanos(),
            )
            .map_err(syscall_error)?;

            fs.disk.file.set_len(DISK_SIZE)?;
        }

        {
            let redoxfs_opt = if fuse {
                Some(RedoxFs::new(&base_partial, &base_dir)?)
            } else {
                None
            };

            if fuse && base_tar.exists() {
                // redoxer in docker was built without kvm, then CI has kvm
                eprintln!("redoxer: extracting {}", name);
                Command::new("tar")
                    .arg("-x")
                    .arg("-p")
                    .arg("-f")
                    .arg(&base_tar)
                    .arg("-C")
                    .arg(&base_dir)
                    .status()
                    .and_then(status_error)?;
            } else {
                let config: redox_installer::Config =
                    toml::from_str(if gui { GUI_TOML } else { BASE_TOML })
                        .map_err(|err| io::Error::new(io::ErrorKind::Other, format!("{}", err)))?;

                let cookbook: Option<&str> = None;
                redox_installer::install(config, &base_dir, cookbook, INSTALL_LIVE_IMAGE, None)
                    .map_err(|err| io::Error::new(io::ErrorKind::Other, format!("{}", err)))?;
            }

            if let Some(mut redoxfs) = redoxfs_opt {
                redoxfs.unmount()?;
            }
        }

        if !fuse {
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

        fs::rename(&base_partial, &base_bin)?;
        fs::remove_dir_all(&base_dir)?;
    }
    Ok(base_bin)
}

fn archive_free_space(
    disk_path: &Path,
    folder_path: &Path,
    bootloader_path: &Path,
    free_space: u64,
) -> io::Result<()> {
    let disk = DiskSparse::create(&disk_path, free_space).map_err(syscall_error)?;

    let bootloader = {
        let mut bootloader = fs::read(bootloader_path)?.to_vec();

        // Pad bootloader to 2 MiB
        while bootloader.len() < BOOTLOADER_SIZE {
            bootloader.push(0);
        }

        bootloader
    };

    let ctime = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let mut fs = FileSystem::create_reserved(
        disk,
        None,
        &bootloader,
        ctime.as_secs(),
        ctime.subsec_nanos(),
    )
    .map_err(syscall_error)?;

    let end_block = fs
        .tx(|tx| {
            // Archive_at root node
            archive_at(tx, folder_path, TreePtr::root())
                .map_err(|err| syscall::Error::new(err.raw_os_error().unwrap()))?;

            // Squash alloc log
            tx.sync(true)?;

            let end_block = tx.header.size() / BLOCK_SIZE;
            /* TODO: Cut off any free blocks at the end of the filesystem
            let mut end_changed = true;
            while end_changed {
                end_changed = false;

                let allocator = fs.allocator();
                let levels = allocator.levels();
                for level in 0..levels.len() {
                    let level_size = 1 << level;
                    for &block in levels[level].iter() {
                        if block < end_block && block + level_size >= end_block {
                            end_block = block;
                            end_changed = true;
                        }
                    }
                }
            }
            */

            // Update header
            tx.header.size = (end_block * BLOCK_SIZE).into();
            tx.header_changed = true;
            tx.sync(false)?;

            Ok(end_block)
        })
        .map_err(syscall_error)?;

    let size = (fs.block + end_block) * BLOCK_SIZE;
    fs.disk.file.set_len(size)?;

    Ok(())
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

fn inner(
    arguments: &[String],
    config: &RedoxerConfig,
    folder_opt: Option<String>,
    gui: bool,
    output_opt: Option<String>,
) -> anyhow::Result<i32> {
    let qemu_binary = config
        .qemu_binary
        .as_deref()
        .unwrap_or("qemu-system-x86_64");

    if !installed(qemu_binary)? {
        eprintln!("redoxer: qemu-system-x86 not found, please install before continuing");
        process::exit(1);
    }
    let kvm = Path::new("/dev/kvm").exists();

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
    let base_bin = base(&bootloader_bin, gui, fuse).context("unable to init base")?;

    let tempdir = tempfile::tempdir().context("unable to create tempdir")?;

    let code = {
        let redoxer_bin = tempdir.path().join("redoxer.bin");
        if fuse {
            Command::new("cp")
                .arg(&base_bin)
                .arg(&redoxer_bin)
                .status()
                .and_then(status_error)
                .context("copy base to redoxer bin failed")?;
        }

        let redoxer_dir = tempdir.path().join("redoxer");
        fs::create_dir_all(&redoxer_dir).context("unable to create redoxer dir")?;

        {
            let redoxfs_opt = if fuse {
                Some(RedoxFs::new(&redoxer_bin, &redoxer_dir).context("unable to init redoxfs")?)
            } else {
                Command::new("tar")
                    .arg("-x")
                    .arg("-p")
                    .arg("--same-owner")
                    .arg("-f")
                    .arg(&base_bin)
                    .arg("-C")
                    .arg(&redoxer_dir)
                    .arg(".")
                    .status()
                    .and_then(status_error)
                    .context("tar failed")?;
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
            fs::write(redoxer_dir.join("etc/redoxerd"), redoxerd_config)
                .context("unable to write redoxerd config")?;

            if let Some(ref folder) = folder_opt {
                eprintln!("redoxer: copying '{}' to '/root'", folder);

                let root_dir = redoxer_dir.join("root");
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
            archive_free_space(&redoxer_bin, &redoxer_dir, &bootloader_bin, DISK_SIZE)?;
        }

        let redoxer_log = tempdir.path().join("redoxer.log");
        let mut command = Command::new(qemu_binary);

        let chardev = format!("file,id=log,path={}", redoxer_log.display());
        let drive = format!("file={},format=raw", redoxer_bin.display());
        #[rustfmt::skip]
        let mut default_args = vec![
            "-cpu", "max",
            "-machine", "q35", 
            "-m", "2048",
            "-smp", "4",
            "-serial", "mon:stdio",
            "-chardev", &chardev,
            "-netdev", "user,id=net0",
            "-device", "isa-debugcon,chardev=log",
            "-device", "isa-debug-exit",
            "-device", "e1000,netdev=net0",
            "-drive", &drive,
        ];
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
