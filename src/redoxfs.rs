use std::{fs, io, thread, time};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{running, status_error};

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
    pub fn mount(&mut self) -> io::Result<()> {
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
    pub fn unmount(&mut self) -> io::Result<()> {
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
