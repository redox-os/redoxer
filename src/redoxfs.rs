use anyhow::{anyhow, bail, Context};
use redoxfs::{BlockAddr, BlockMeta, DiskFile, FileSystem};
use std::fs::File;
use std::os::unix::fs::MetadataExt;
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
            return Err(io::Error::other("directory was already mounted"));
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
                return Err(io::Error::other("directory was still mounted"));
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
        redox_installer::with_redoxfs_mount(fs, Some(base_dir), move |base_dir| {
            if base_tar.exists() {
                // redoxer in docker was built without FUSE, then CI has FUSE
                eprintln!("redoxer: extracting archive");
                extract_tar(base_tar, base_dir)?;
            } else {
                run_install_to_dir(config, base_dir)?;
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
        .map_err(|err| io::Error::other(format!("{}", err)))?;
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

pub(crate) fn extract_tar(tar_file: &Path, dest_dir: &Path) -> anyhow::Result<()> {
    Command::new("tar")
        .arg("-x")
        .arg("-p")
        .arg("--same-owner")
        .arg("-f")
        .arg(tar_file)
        .arg("-C")
        .arg(dest_dir)
        .arg(".")
        .status()
        .and_then(status_error)
        .context("tar extract failed")?;
    Ok(())
}

pub(crate) fn shrink_disk(disk_path: &Path) -> anyhow::Result<()> {
    let shrink_size = {
        let mut fs = open_fs(disk_path)?;
        let (old_size, new_size) = resize(&mut fs, true).map_err(|e| anyhow!(e))?;
        if old_size > new_size {
            Some(old_size - new_size)
        } else {
            None
        }
    };

    if let Some(shrink_size) = shrink_size {
        let f = open_disk(disk_path)?.file;
        let size = f
            .metadata()
            .context("Unable to get disk file metadata")?
            .size();
        f.set_len(size - shrink_size)
            .context("Unable to shrink disk file metadata")?;
    }

    Ok(())
}

pub(crate) fn expand_disk(disk_path: &Path, desired_size: u64) -> anyhow::Result<()> {
    {
        let disk = open_disk(disk_path)?;
        let size = disk
            .file
            .metadata()
            .context("Unable to get disk file metadata")?
            .size();
        if size < desired_size {
            disk.file
                .set_len(desired_size)
                .context("Unable to expand disk file metadata")?;
        } else {
            return Ok(());
        };
    }

    {
        let mut fs = open_fs(disk_path)?;
        resize(&mut fs, false).map_err(|e| anyhow!(e))?;
    }

    Ok(())
}

fn open_fs(disk_path: &Path) -> anyhow::Result<FileSystem<DiskFile>> {
    let disk = open_disk(disk_path)?;
    let fs = match FileSystem::open(disk, None, None, true) {
        Ok(fs) => fs,
        Err(err) => {
            bail!(
                "redoxfs-resize: failed to open filesystem on {}: {}",
                disk_path.display(),
                err
            );
        }
    };
    Ok(fs)
}

fn open_disk(disk_path: &Path) -> anyhow::Result<DiskFile> {
    let disk = match DiskFile::open(disk_path) {
        Ok(disk) => disk,
        Err(err) => {
            bail!(
                "redoxfs-resize: failed to open disk image {}: {}",
                disk_path.display(),
                err
            );
        }
    };
    Ok(disk)
}

// copied from redoxfs-resize
fn resize<D: redoxfs::Disk>(fs: &mut FileSystem<D>, shrink: bool) -> Result<(u64, u64), String> {
    let disk_size = fs
        .disk
        .size()
        .map_err(|err| format!("failed to read disk size: {}", err))?;

    // Find contiguous free region
    //TODO: better error management
    let mut last_free = None;
    let mut last_end = 0;
    fs.tx(|tx| {
        let mut alloc_ptr = tx.header.alloc;
        while !alloc_ptr.is_null() {
            let alloc = tx.read_block(alloc_ptr)?;
            alloc_ptr = alloc.data().prev;
            for entry in alloc.data().entries.iter() {
                let count = entry.count();
                if count <= 0 {
                    continue;
                }
                let end = entry.index() + count as u64;
                if end > last_end {
                    last_free = Some(*entry);
                    last_end = end;
                }
            }
        }
        Ok(())
    })
    .map_err(|err| format!("failed to read alloc log: {}", err))?;

    let old_size = fs.header.size();
    let min_size = if let Some(entry) = last_free {
        entry.index() * redoxfs::BLOCK_SIZE
    } else {
        old_size
    };
    let max_size = disk_size - (fs.block * redoxfs::BLOCK_SIZE);

    let new_size = if shrink { min_size } else { max_size };

    let old_blocks = old_size / redoxfs::BLOCK_SIZE;
    let new_blocks = new_size / redoxfs::BLOCK_SIZE;
    let (start, end, shrink) = if new_size == old_size {
        return Ok((old_size, new_size));
    } else if new_size < old_size {
        (new_blocks, old_blocks, true)
    } else {
        (old_blocks, new_blocks, false)
    };

    // Allocate or deallocate blocks as needed
    unsafe {
        let allocator = fs.allocator_mut();
        for index in start..end {
            if shrink {
                //TODO: replace assert with error?
                let addr = BlockAddr::new(index, BlockMeta::default());
                assert_eq!(allocator.allocate_exact(addr), Some(addr));
            } else {
                let addr = BlockAddr::new(index, BlockMeta::default());
                allocator.deallocate(addr);
            }
        }
    }

    fs.tx(|tx| {
        // Update header
        tx.header.size = new_size.into();
        tx.header_changed = true;

        // Sync with squash
        tx.sync(true)?;

        Ok(())
    })
    .map_err(|err| format!("transaction failed: {}", err))?;

    Ok((old_size, new_size))
}
