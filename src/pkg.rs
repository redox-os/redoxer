use pkg::{callback::IndicatifCallback, Library, PackageName};
use std::{
    cell::RefCell,
    env, fs, io,
    os::unix,
    path::{Path, PathBuf},
    process,
    rc::Rc,
};

use crate::target;

pub const DEFAULT_PKG_SOURCE: &str = "https://static.redox-os.org/pkg";

enum PkgCommand {
    Install,
    Update,
    Remove,
}

fn pkg_inner(
    sysroot: PathBuf,
    source: String,
    cmd: PkgCommand,
    pkgs: Vec<PackageName>,
) -> io::Result<()> {
    let callback = IndicatifCallback::new();

    if !sysroot.is_dir() {
        println!("redoxer: building pkg sysroot");
        fs::create_dir_all(sysroot.join("etc/pkg.d"))?;
        fs::create_dir(sysroot.join("usr"))?;
        fs::write(sysroot.join("etc/pkg.d/10_redoxer"), source)?;
        for folder in &["bin", "include", "lib", "share"] {
            fs::create_dir(sysroot.join("usr").join(folder))?;
            unix::fs::symlink(format!("usr/{folder}"), sysroot.join(folder))?;
        }
    }

    let mut library = Library::new(sysroot, target(), Rc::new(RefCell::new(callback))).unwrap();

    match cmd {
        PkgCommand::Install => library.install(pkgs),
        PkgCommand::Update => library.update(pkgs),
        PkgCommand::Remove => library.uninstall(pkgs),
    }
    .unwrap();

    library
        .apply()
        .map_err(|e: pkg::Error| {
            eprintln!("Unable to complete pkg action: {e}");
            process::exit(1);
        })
        .unwrap();

    Ok(())
}

pub fn main(args: &[String]) {
    let args: Vec<String> = args.iter().cloned().skip(2).collect();

    let mut i = 0;
    let mut pkgs = Vec::new();
    let mut cmd = PkgCommand::Install;
    let mut cmd_set = false;
    while i < args.len() {
        match (args[i].as_str(), cmd_set) {
            ("install", false) => {
                cmd = PkgCommand::Install;
                cmd_set = true;
            }
            ("update", false) => {
                cmd = PkgCommand::Update;
                cmd_set = true;
            }
            ("remove", false) => {
                cmd = PkgCommand::Remove;
                cmd_set = true;
            }
            ("--help", _) => {
                pkg_usage();
            }
            (arg, _) => {
                pkgs.push(arg);
            }
        }
        i += 1;
    }

    if pkgs.is_empty() {
        pkg_usage();
    }

    let sysroot = get_sysroot()
        .or_else(get_cargo_sysroot_default_path)
        .expect("Please define REDOXER_SYSROOT as destination to install packages");

    let source = env::var("REDOXER_PKG_SOURCE").unwrap_or(DEFAULT_PKG_SOURCE.to_string());

    let pkgs = pkgs
        .iter()
        .map(|p| PackageName::new(p.to_string()).expect(&format!("Invalid pkg name: {p}")))
        .collect();

    match pkg_inner(sysroot, source, cmd, pkgs) {
        Ok(_) => {
            process::exit(0);
        }
        Err(err) => {
            eprintln!("redoxer toolchain: {}", err);
            process::exit(1);
        }
    }
}

pub fn get_sysroot() -> Option<PathBuf> {
    env::var("REDOXER_SYSROOT")
        .ok()
        .map(|p| Path::new(&p).to_owned())
        .or_else(|| {
            let path = get_cargo_sysroot_default_path()?;
            if path.join("lib").is_dir() {
                return Some(path);
            }
            None
        })
}

fn get_cargo_sysroot_default_path() -> Option<PathBuf> {
    if Path::new("Cargo.toml").is_file() {
        Some(Path::new(&format!("target/{}/sysroot", target())).to_path_buf())
    } else {
        None
    }
}

fn pkg_usage() {
    eprintln!("redoxer pkg [install|remove|update] pkg-1 pkg-2 ...");
    eprintln!(" arguments:");
    eprintln!("   [install|remove|update]  whether to install, update, or remove pkg (optional, default is install)");
    eprintln!();
    eprintln!(" environment variables:");
    eprintln!("   REDOXER_SYSROOT          where to install sysroot (required when no Cargo.toml)");
    eprintln!("   REDOXER_PKG_SOURCE       whether to install custom source instead of {DEFAULT_PKG_SOURCE}");
    eprintln!();
    process::exit(0);
}
