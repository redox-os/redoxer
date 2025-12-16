use anyhow::Context;
use redoxfs::{DiskFile, FileSystem};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{channel, TryRecvError};
use std::{fs, io, thread, time};

use crate::status_error;

pub struct RedoxFs {
    image: PathBuf,
    dir: PathBuf,
}

pub(crate) fn syscall_error(err: syscall::Error) -> io::Error {
    io::Error::from_raw_os_error(err.errno)
}

impl RedoxFs {
    pub fn new<P: AsRef<Path>, Q: AsRef<Path>>(image: P, dir: Q) -> io::Result<Self> {
        let image = image.as_ref().to_owned();
        let dir = fs::canonicalize(dir)?;
        let mut s = Self { image, dir };
        s.mount()?;
        Ok(s)
    }

    pub fn mount(&mut self) -> io::Result<()> {
        if self.mounted()? {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "directory was already mounted",
            ));
        }

        let (tx, rx) = channel();

        let disk = DiskFile::open(&self.image).map_err(syscall_error)?;
        let fs = FileSystem::open(disk, None, None, true).map_err(syscall_error)?;
        let dir = self.dir.clone();
        thread::spawn(move || {
            let _ = tx.send(redoxfs::mount(fs, dir, |_| {}));
        });

        while !self.mounted()? {
            match rx.try_recv() {
                Ok(res) => match res {
                    Ok(()) => {
                        return Err(io::Error::new(
                            io::ErrorKind::NotConnected,
                            "redoxfs thread exited early",
                        ))
                    }
                    Err(err) => return Err(err),
                },
                Err(err) => match err {
                    TryRecvError::Empty => (),
                    TryRecvError::Disconnected => {
                        return Err(io::Error::new(
                            io::ErrorKind::NotConnected,
                            "redoxfs thread did not send a result",
                        ))
                    }
                },
            }
            thread::sleep(time::Duration::from_millis(1));
        }

        Ok(())
    }

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
                    "directory was still mounted",
                ));
            }
        }

        Ok(())
    }

    pub fn mounted(&self) -> io::Result<bool> {
        use proc_mounts::MountIter;

        for mount_res in MountIter::new()? {
            let mount = mount_res?;
            if mount.dest == self.dir {
                return Ok(true);
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

pub fn run_install_mount(
    config: redox_installer::Config,
    bootloader_bin: &Path,
    bootloader_uefi: bool,
    base_size: u64,
    base_tar: &Path,
    base_dir: &Path,
    base_bin: &PathBuf,
) -> Result<(), anyhow::Error> {
    let (bootloader_bios, bootloader_efi) =
        read_bootloader(base_bin, bootloader_bin, bootloader_uefi, base_size)?;
    let disk_option = redox_installer::DiskOption {
        bootloader_bios: &bootloader_bios,
        bootloader_efi: &bootloader_efi,
        password_opt: None,
        efi_partition_size: None,
        skip_partitions: false,
    };
    redox_installer::with_whole_disk(base_bin, &disk_option, move |fs| {
        redox_installer::with_redoxfs_mount(fs, Some(&base_dir), move |base_dir| {
            if base_tar.exists() {
                // redoxer in docker was built without FUSE, then CI has FUSE
                eprintln!("redoxer: extracting archive");
                extract_tar(base_tar, base_dir)?;
            } else {
                run_install_to_dir(config, &base_dir)?;
            }

            Ok(())
        })
    })
    .context("Unable to create image from redoxfs-mount")?;
    Ok(())
}

pub fn archive_image(
    disk_path: &Path,
    folder_path: &Path,
    bootloader_bin: &Path,
    use_uefi: bool,
    free_space: u64,
) -> anyhow::Result<()> {
    let (bootloader_bios, bootloader_efi) =
        read_bootloader(disk_path, bootloader_bin, use_uefi, free_space)?;
    let disk_option = redox_installer::DiskOption {
        bootloader_bios: &bootloader_bios,
        bootloader_efi: &bootloader_efi,
        password_opt: None,
        efi_partition_size: None,
        skip_partitions: false,
    };
    redox_installer::with_whole_disk(disk_path, &disk_option, move |fs| {
        redox_installer::with_redoxfs_ar(fs, Some(folder_path), move |_| Ok(()))
    })
    .context("Unable to create image from redoxfs-ar")?;

    Ok(())
}

pub fn run_install_to_dir(config: redox_installer::Config, base_dir: &Path) -> io::Result<()> {
    redox_installer::install(config, base_dir)
        .map_err(|err| io::Error::new(io::ErrorKind::Other, format!("{}", err)))?;
    Ok(())
}

fn read_bootloader(
    disk_path: &Path,
    bootloader_bin: &Path,
    use_uefi: bool,
    free_space: u64,
) -> Result<(Vec<u8>, Vec<u8>), anyhow::Error> {
    {
        let file = File::create(disk_path)?;
        file.set_len(free_space)?;
    }
    let bootloader_bios = if use_uefi {
        Vec::new()
    } else {
        fs::read(bootloader_bin)?.to_vec()
    };
    let bootloader_efi = if use_uefi {
        fs::read(bootloader_bin)?.to_vec()
    } else {
        Vec::new()
    };
    Ok((bootloader_bios, bootloader_efi))
}

pub(crate) fn extract_tar(tar_file: &Path, dest_dir: &Path) -> Result<(), anyhow::Error> {
    Command::new("tar")
        .arg("-x")
        .arg("-p")
        .arg("--same-owner")
        .arg("-f")
        .arg(&tar_file)
        .arg("-C")
        .arg(dest_dir)
        .arg(".")
        .status()
        .and_then(status_error)
        .context("tar extract failed")?;
    Ok(())
}
