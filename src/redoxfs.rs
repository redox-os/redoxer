use redoxfs::{DiskFile, FileSystem};
use std::{fs, io, thread, time};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{channel, TryRecvError};

use crate::{status_error, syscall_error};

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

        let (tx, rx) = channel();

        let disk = DiskFile::open(&self.image).map_err(syscall_error)?;
        let fs = FileSystem::open(disk, None, None, true).map_err(syscall_error)?;
        let dir = self.dir.clone();
        thread::spawn(move || {
            tx.send(redoxfs::mount(
                fs,
                dir,
                |_| {}
            )).unwrap();
        });

        while ! self.mounted()? {
            match rx.try_recv() {
                Ok(res) => match res {
                    Ok(()) => return Err(io::Error::new(
                        io::ErrorKind::NotConnected,
                        "redoxfs thread exited early"
                    )),
                    Err(err) => return Err(err),
                },
                Err(err) => match err {
                    TryRecvError::Empty => (),
                    TryRecvError::Disconnected => return Err(io::Error::new(
                        io::ErrorKind::NotConnected,
                        "redoxfs thread did not send a result"
                    )),
                },
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
