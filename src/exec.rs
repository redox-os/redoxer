use std::{env, fs, io};
use std::io::{Seek, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Command};

use crate::{installed, redoxer_dir, status_error};
use crate::redoxfs::RedoxFs;

static BASE_TOML: &'static str = include_str!("../res/base.toml");
static GUI_TOML: &'static str = include_str!("../res/gui.toml");

fn bootloader() -> io::Result<PathBuf> {
    let bootloader_bin = redoxer_dir().join("bootloader.bin");
    if ! bootloader_bin.is_file() {
        eprintln!("redoxer: building bootloader");

        let bootloader_dir = redoxer_dir().join("bootloader");
        if bootloader_dir.is_dir() {
            fs::remove_dir_all(&bootloader_dir)?;
        }
        fs::create_dir_all(&bootloader_dir)?;

        let mut config = redox_installer::Config::default();
        config.packages.insert("bootloader".to_string(), Default::default());
        let cookbook: Option<&str> = None;
        redox_installer::install(config, &bootloader_dir, cookbook).map_err(|err| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("{}", err)
            )
        })?;

        fs::rename(&bootloader_dir.join("bootloader"), &bootloader_bin)?;
    }
    Ok(bootloader_bin)
}

fn base(bootloader_bin: &Path, gui: bool) -> io::Result<PathBuf> {
    let name = if gui { "gui" } else { "base" };

    let base_bin = redoxer_dir().join(format!("{}.bin", name));
    if ! base_bin.is_file() {
        eprintln!("redoxer: building {}", name);

        let base_dir = redoxer_dir().join(name);
        if base_dir.is_dir() {
            fs::remove_dir_all(&base_dir)?;
        }
        fs::create_dir_all(&base_dir)?;

        let base_partial = redoxer_dir().join(format!("{}.bin.partial", name));
        Command::new("truncate")
            .arg("--size=4G")
            .arg(&base_partial)
            .status()
            .and_then(status_error)?;

        Command::new("redoxfs-mkfs")
            .arg(&base_partial)
            .arg(&bootloader_bin)
            .status()
            .and_then(status_error)?;

        {
            let mut redoxfs = RedoxFs::new(&base_partial, &base_dir)?;

            let config: redox_installer::Config = toml::from_str(
                if gui { GUI_TOML } else { BASE_TOML }
            ).map_err(|err| {
                io::Error::new(
                    io::ErrorKind::Other,
                    format!("{}", err)
                )
            })?;

            let cookbook: Option<&str> = None;
            redox_installer::install(config, &base_dir, cookbook).map_err(|err| {
                io::Error::new(
                    io::ErrorKind::Other,
                    format!("{}", err)
                )
            })?;

            redoxfs.unmount()?;
        }

        fs::rename(&base_partial, &base_bin)?;
    }
    Ok(base_bin)
}

fn inner(arguments: &[String], folder_opt: Option<String>, gui: bool) -> io::Result<i32> {
    if ! installed("kvm")? {
        eprintln!("redoxer: kvm not found, please install before continuing");
        process::exit(1);
    }

    if ! installed("redoxfs")? {
        eprintln!("redoxer: redoxfs not found, installing with cargo");
        Command::new("cargo")
            .arg("install")
            .arg("redoxfs")
            .status()
            .and_then(status_error)?;
    }

    let bootloader_bin = bootloader()?;
    let base_bin = base(&bootloader_bin, gui)?;

    let tempdir = tempfile::tempdir()?;

    let code = {
        let redoxer_bin = tempdir.path().join("redoxer.bin");
        Command::new("cp")
            .arg(&base_bin)
            .arg(&redoxer_bin)
            .status()
            .and_then(status_error)?;

        let redoxer_dir = tempdir.path().join("redoxer");
        fs::create_dir_all(&redoxer_dir)?;

        {
            let mut redoxfs = RedoxFs::new(&redoxer_bin, &redoxer_dir)?;

            let mut redoxerd_config = String::new();
            for arg in arguments.iter() {
                // Replace absolute path to folder with /root in command name
                // TODO: make this activated by a flag
                if let Some(ref folder) = folder_opt {
                    let folder_canonical_path = fs::canonicalize(&folder)?;
                    let folder_canonical = folder_canonical_path.to_str().ok_or(io::Error::new(
                        io::ErrorKind::Other,
                        "folder path is not valid UTF-8"
                    ))?;
                    if arg.starts_with(&folder_canonical) {
                        let arg_replace = arg.replace(folder_canonical, "/root");
                        eprintln!("redoxer: replacing '{}' with '{}' in arguments", arg, arg_replace);
                        redoxerd_config.push_str(&arg_replace);
                        redoxerd_config.push('\n');
                        continue;
                    }
                }

                redoxerd_config.push_str(&arg);
                redoxerd_config.push('\n');
            }
            fs::write(redoxer_dir.join("etc/redoxerd"), redoxerd_config)?;

            if let Some(ref folder) = folder_opt {
                eprintln!("redoxer: copying '{}' to '/root'", folder);

                let root_dir = redoxer_dir.join("root");
                Command::new("cp")
                    .arg("--dereference")
                    .arg("--no-target-directory")
                    .arg("--preserve=mode,timestamps")
                    .arg("--recursive")
                    .arg(&folder)
                    .arg(&root_dir)
                    .status()
                    .and_then(status_error)?;
            }

            redoxfs.unmount()?;
        }

        // Set default bootloader configuration
        if gui {
            let mut f = fs::OpenOptions::new()
                .write(true)
                .open(&redoxer_bin)?;

            // Configuration is stored in the third sector
            f.seek(io::SeekFrom::Start(512 * 3))?;

            // Width and height are stored as two u16 values
            let width = 1024;
            let height = 768;
            f.write(&[
                width as u8,
                (width >> 8) as u8,
                height as u8,
                (height >> 8) as u8,
            ])?;
        }

        let redoxer_log = tempdir.path().join("redoxer.log");
        let mut command = Command::new("kvm");
        command
            .arg("-cpu").arg("host")
            .arg("-machine").arg("q35")
            .arg("-m").arg("2048")
            .arg("-smp").arg("4")
            .arg("-serial").arg("mon:stdio")
            .arg("-chardev").arg(format!("file,id=log,path={}", redoxer_log.display()))
            .arg("-device").arg("isa-debugcon,chardev=log")
            .arg("-device").arg("isa-debug-exit")
            .arg("-netdev").arg("user,id=net0")
            .arg("-device").arg("e1000,netdev=net0")
            .arg("-drive").arg(format!("file={},format=raw", redoxer_bin.display()));
        if ! gui {
            command
                .arg("-nographic")
                .arg("-vga").arg("none");
        }

        let status = command.status()?;

        eprintln!();

        let code = match status.code() {
            Some(51) => {
                eprintln!("## redoxer (success) ##");
                0
            },
            Some(53) => {
                eprintln!("## redoxer (failure) ##");
                1
            },
            _ => {
                eprintln!("## redoxer (failure, qemu exit status {:?} ##", status);
                2
            }
        };

        print!("{}", fs::read_to_string(&redoxer_log)?);

        code
    };

    tempdir.close()?;

    Ok(code)
}

fn usage() {
    eprintln!("redoxer exec [-f|--folder folder] [-g|--gui] [-h|--help] [--] <command> [arguments]...");
    process::exit(1);
}

pub fn main() {
    // Matching flags
    let mut matching = true;
    // Folder to copy
    let mut folder_opt = None;
    // Run with GUI
    let mut gui = false;
    // Arguments to pass to command
    let mut arguments = Vec::new();

    let mut args = env::args().skip(2);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-f" | "--folder" if matching => match args.next() {
                Some(folder) => {
                    folder_opt = Some(folder);
                },
                None => {
                    usage();
                },
            },
            "-g" | "--gui" if matching => {
                gui = true;
            },
            // TODO: argument for replacing the folder path with /root when found in arguments
            "-h" | "--help" if matching => {
                usage();
            },
            // TODO: "-p" | "--package"
            "--" if matching => {
                matching = false;
            }
            _ => {
                matching = false;
                arguments.push(arg);
            }
        }
    }

    if arguments.is_empty() {
        usage();
    }

    match inner(&arguments, folder_opt, gui) {
        Ok(code) => {
            process::exit(code);
        },
        Err(err) => {
            eprintln!("redoxer exec: {}", err);
            process::exit(3);
        }
    }
}
