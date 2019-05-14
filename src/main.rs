use std::{env, fs, io, thread, time};
use std::path::{Path, PathBuf};
use std::process::{self, Command, ExitStatus, Stdio};

static BASE_TOML: &'static str = include_str!("../res/base.toml");

pub struct RedoxFs {
    image: PathBuf,
    dir: PathBuf,
}

impl RedoxFs {
    pub fn new<P: AsRef<Path>, Q: AsRef<Path>>(image: P, dir: Q) -> io::Result<Self> {
        let image = image.as_ref().to_owned();
        let dir = fs::canonicalize(dir)?;
        let mut s = Self {
            image,
            dir
        };
        s.mount()?;
        Ok(s)
    }

    //TODO: Confirm capabilities on other OSes
    #[cfg(target_os = "linux")]
    fn mount(&mut self) -> io::Result<()> {
        if self.mounted()? {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "directory was already mounted"
            ));
        }

        Command::new("redoxfs")
            .arg(&self.image)
            .arg(&self.dir)
            .status()
            .and_then(status_error)?;

        while ! self.mounted()? {
            if ! running("redoxfs")? {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "redoxfs process exited"
                ));
            }
            thread::sleep(time::Duration::from_millis(1));
        }

        Ok(())
    }

    //TODO: Confirm capabilities on other OSes
    #[cfg(target_os = "linux")]
    fn unmount(&mut self) -> io::Result<()> {
        if self.mounted()? {
            Command::new("fusermount")
                .arg("-u")
                .arg(&self.dir)
                .status()
                .and_then(status_error)?;

            if self.mounted()? {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "directory was still mounted"
                ));
            }
        }

        Ok(())
    }

    //TODO: Confirm capabilities on other OSes
    #[cfg(target_os = "linux")]
    pub fn mounted(&self) -> io::Result<bool> {
        use proc_mounts::MountIter;

        for mount_res in MountIter::new()? {
            let mount = mount_res?;
            if mount.dest == self.dir {
                return Ok(true)
            }
        }

        Ok(false)
    }
}

impl Drop for RedoxFs {
    fn drop(&mut self) {
        if let Err(err) = self.unmount() {
            panic!(
                "RedoxFs::drop: failed to unmount '{}': {}",
                self.dir.display(),
                err
            );
        }
    }
}

fn status_error(status: ExitStatus) -> io::Result<()> {
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Other,
            format!("{}", status)
        ))
    }
}

//TODO: Confirm capabilities on other OSes
#[cfg(target_os = "linux")]
fn installed(program: &str) -> io::Result<bool> {
    Command::new("which")
        .arg(program)
        .stdout(Stdio::null())
        .status()
        .map(|x| x.success())
}

//TODO: Confirm capabilities on other OSes
#[cfg(target_os = "linux")]
fn running(program: &str) -> io::Result<bool> {
    Command::new("pgrep")
        .arg(program)
        .stdout(Stdio::null())
        .status()
        .map(|x| x.success())
}

fn redoxer_dir() -> PathBuf {
    dirs::home_dir().unwrap_or(PathBuf::from("."))
        .join(".redoxer")
}

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

fn base(bootloader_bin: &Path) -> io::Result<PathBuf> {
    let base_bin = redoxer_dir().join("base.bin");
    if ! base_bin.is_file() {
        eprintln!("redoxer: building base");

        let base_dir = redoxer_dir().join("base");
        if base_dir.is_dir() {
            fs::remove_dir_all(&base_dir)?;
        }
        fs::create_dir_all(&base_dir)?;

        let base_partial = redoxer_dir().join("base.bin.partial");
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

            let config: redox_installer::Config = toml::from_str(BASE_TOML).unwrap(); //TODO
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

//TODO: Rewrite with hyper or reqwest, tar-rs, sha2, and some gzip crate?
fn download<P: AsRef<Path>>(url: &str, path: P) -> io::Result<()> {
    Command::new("curl")
        .arg("--proto").arg("=https")
        .arg("--tlsv1.2")
        .arg("--fail")
        .arg("--output").arg(path.as_ref())
        .arg(url)
        .status()
        .and_then(status_error)
}

//TODO: Rewrite with hyper or reqwest, tar-rs, sha2, and some gzip crate?
fn shasum<P: AsRef<Path>>(path: P) -> io::Result<bool> {
    let parent = path.as_ref().parent().ok_or(
        io::Error::new(
            io::ErrorKind::Other,
            "shasum path had no parent"
        )
    )?;
    Command::new("sha256sum")
        .arg("--check")
        .arg("--ignore-missing")
        .arg("--quiet")
        .arg(path.as_ref())
        .current_dir(parent)
        .status()
        .map(|status| status.success())
}

//TODO: Rewrite with hyper or reqwest, tar-rs, sha2, and some gzip crate?
fn prefix() -> io::Result<PathBuf> {
    let url = "https://static.redox-os.org/toolchain/x86_64-unknown-redox";
    let toolchain_dir = redoxer_dir().join("toolchain");
    let prefix_dir = toolchain_dir.join("prefix");
    if ! prefix_dir.is_dir() {
        println!("redoxer: building prefix");

        if toolchain_dir.is_dir() {
            fs::remove_dir_all(&toolchain_dir)?;
        }
        fs::create_dir_all(&toolchain_dir)?;

        let shasum_file = toolchain_dir.join("SHA256SUM");
        download(&format!("{}/SHA256SUM", url), &shasum_file)?;

        let prefix_tar = toolchain_dir.join("relibc-install.tar.gz");
        download(&format!("{}/relibc-install.tar.gz", url), &prefix_tar)?;

        if ! shasum(&shasum_file)? {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "shasum invalid"
            ));
        }

        let prefix_partial = redoxer_dir().join("prefix.partial");
        fs::create_dir_all(&prefix_partial)?;

        Command::new("tar")
            .arg("--extract")
            .arg("--file").arg(&prefix_tar)
            .arg("-C").arg(&prefix_partial)
            .arg(".")
            .status()
            .and_then(status_error)?;

        fs::rename(&prefix_partial, &prefix_dir)?;
    }

    Ok(prefix_dir)
}

fn inner(arguments: &[String], folder_opt: Option<String>) -> io::Result<i32> {
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
    let base_bin = base(&bootloader_bin)?;
    let prefix_dir = prefix()?;

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
                redoxerd_config.push_str(&arg);
                redoxerd_config.push('\n');
            }
            fs::write(redoxer_dir.join("etc/redoxerd"), redoxerd_config)?;

            if let Some(folder) = folder_opt {
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

        let redoxer_log = tempdir.path().join("redoxer.log");
        let status = Command::new("kvm")
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
            .arg("-nographic")
            .arg("-vga").arg("none")
            .arg("-drive").arg(format!("file={},format=raw", redoxer_bin.display()))
            .status()?;

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
    eprintln!("redoxer [-f|--folder folder] [--] <command> [arguments]...");
    process::exit(1);
}

fn main() {
    // Matching flags
    let mut matching = true;
    // Folder to copy
    let mut folder_opt = None;
    // Arguments to pass to command
    let mut arguments = Vec::new();

    let mut args = env::args().skip(1);
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
            "-h" | "--help" if matching => {
                usage();
            },
            //TODO: "-p" | "--package"
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

    match inner(&arguments, folder_opt) {
        Ok(code) => {
            process::exit(code);
        },
        Err(err) => {
            eprintln!("redoxer: {}", err);
            process::exit(3);
        }
    }
}
