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

fn bootloader() -> io::Result<&'static Path> {
    let bootloader_bin = Path::new("build/bootloader.bin");
    if ! bootloader_bin.is_file() {
        eprintln!("redoxer: building bootloader");

        let bootloader_dir = Path::new("build/bootloader");
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

        fs::rename(&bootloader_dir.join("bootloader"), bootloader_bin)?;
    }
    Ok(bootloader_bin)
}

fn base(bootloader_bin: &Path) -> io::Result<&'static Path> {
    let base_bin = Path::new("build/base.bin");
    if ! base_bin.is_file() {
        eprintln!("redoxer: building base");

        let base_dir = Path::new("build/base");
        if base_dir.is_dir() {
            fs::remove_dir_all(&base_dir)?;
        }
        fs::create_dir_all(&base_dir)?;

        let base_partial = Path::new("build/base.bin.partial");
        Command::new("dd")
            .arg("if=/dev/zero")
            .arg(format!("of={}", base_partial.display()))
            .arg("bs=1M")
            .arg("count=256")
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

fn inner() -> io::Result<()> {
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
    let base_bin = base(bootloader_bin)?;

    let tempdir = tempfile::tempdir()?;

    {
        let redoxer_bin = tempdir.path().join("redoxer.bin");
        fs::copy(&base_bin, &redoxer_bin)?;

        let redoxer_dir = tempdir.path().join("redoxer");
        fs::create_dir_all(&redoxer_dir)?;

        {
            let mut redoxfs = RedoxFs::new(&redoxer_bin, &redoxer_dir)?;

            //TODO: Use redoxerd package
            fs::copy(
                "daemon/target/x86_64-unknown-redox/release/redoxerd",
                redoxer_dir.join("bin/redoxerd")
            )?;

            let mut redoxerd_config = String::new();
            for (i, arg) in env::args().skip(1).enumerate() {
                if i == 0 && arg.contains("/") {
                    let name = arg.split("/").last().unwrap();
                    fs::copy(
                        &arg,
                        redoxer_dir.join("bin").join(name)
                    )?;
                    redoxerd_config.push_str(&name);
                    redoxerd_config.push('\n');
                } else {
                    redoxerd_config.push_str(&arg);
                    redoxerd_config.push('\n');
                }
            }
            fs::write(redoxer_dir.join("etc/redoxerd"), redoxerd_config)?;

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

        match status.code() {
            Some(51) => {
                eprintln!("## redoxer (success) ##");
            },
            Some(53) => {
                eprintln!("## redoxer (failure) ##");
                //TODO: Return error
            },
            _ => {
                eprintln!("## redoxer (failure, qemu exit status {:?} ##", status);
                //TODO: Return error
            }
        }

        print!("{}", fs::read_to_string(&redoxer_log)?);
    }

    tempdir.close()?;

    Ok(())
}

fn main() {
    match inner() {
        Ok(()) => (),
        Err(err) => {
            eprintln!("redoxer: {}", err);
            process::exit(1)
        }
    }
}
