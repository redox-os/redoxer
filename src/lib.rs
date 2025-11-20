use std::{io, path, process};

pub(crate) use self::toolchain::toolchain;

mod cargo;
mod env;
mod exec;
mod pkg;
mod redoxfs;
mod toolchain;

const SUPPORTED_TARGETS: &'static [&'static str] = &[
    "x86_64-unknown-redox",
    "aarch64-unknown-redox",
    "i586-unknown-redox",
    "i686-unknown-redox",
    "riscv64gc-unknown-redox",
];

fn installed(program: &str) -> io::Result<bool> {
    process::Command::new("which")
        .arg(program)
        .stdout(process::Stdio::null())
        .status()
        .map(|x| x.success())
}

fn redoxer_dir() -> path::PathBuf {
    dirs::home_dir()
        .unwrap_or(path::PathBuf::from("."))
        .join(".redoxer")
        .join(target())
}

fn status_error(status: process::ExitStatus) -> io::Result<()> {
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::Other, format!("{}", status)))
    }
}

fn syscall_error(err: syscall::Error) -> io::Error {
    io::Error::from_raw_os_error(err.errno)
}

fn usage() {
    eprintln!("redoxer ar - run AR GNU compiler with Redox target");
    eprintln!("redoxer bench - cargo bench with Redox target in Redox VM");
    eprintln!("redoxer build - cargo build with Redox target");
    eprintln!("redoxer cc - run C GNU compiler with Redox target");
    eprintln!("redoxer check - cargo check with Redox target");
    eprintln!("redoxer cxx - run C++ GNU compiler with Redox target");
    eprintln!("redoxer doc - cargo doc with Redox target");
    eprintln!("redoxer env - execute a command in cross-compilation environment");
    eprintln!("redoxer exec - execute a command in Redox VM");
    eprintln!("redoxer fetch - cargo fetch with Redox target");
    eprintln!("redoxer install - cargo install with Redox target");
    eprintln!("redoxer pkg - install sysroot for native dependencies");
    eprintln!("redoxer run - cargo run with Redox target in Redox VM");
    eprintln!("redoxer rustc - cargo rustc with Redox target");
    eprintln!("redoxer test - cargo test with Redox target in Redox VM");
    eprintln!("redoxer toolchain - install toolchain");
    process::exit(1);
}

pub fn target() -> &'static str {
    let target_from_env = std::env::var("TARGET").unwrap_or("".to_string());

    // Allow compilation for host if explicitly requested
    if target_from_env == host_target() {
        return host_target();
    }

    let index = if SUPPORTED_TARGETS.contains(&&*target_from_env) == true {
        SUPPORTED_TARGETS
            .iter()
            .position(|t| **t == target_from_env)
            .unwrap()
            .into()
    } else {
        0usize
    };

    SUPPORTED_TARGETS[index]
}

pub fn gnu_target() -> &'static str {
    match target() {
        "riscv64gc-unknown-redox" => "riscv64-unknown-redox",
        "aarch64-unknown-linux-gnu" => "aarch64-linux-gnu",
        "x86_64-unknown-linux-gnu" => "x86_64-linux-gnu",
        rust_target => rust_target,
    }
}

pub fn host_target() -> &'static str {
    let os = if cfg!(target_os = "linux") {
        "linux-gnu"
    } else if cfg!(target_os = "redox") {
        "redox"
    } else {
        ""
    };
    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        ""
    };
    match (arch, os) {
        ("x86_64", "linux-gnu") => "x86_64-unknown-linux-gnu",
        ("aarch64", "linux-gnu") => "aarch64-unknown-linux-gnu",
        ("x86_64", "redox") => "x86_64-unknown-redox",
        ("aarch64", "redox") => "aarch64-unknown-redox",
        _ => panic!("Unsupported host OS/ARCH!"),
    }
}

pub fn main(args: &[String]) {
    match args.get(1) {
        Some(arg) => match arg.as_str() {
            "bench" | "build" | "check" | "doc" | "fetch" => cargo::main(args),
            "install" | "run" | "rustc" | "test" => cargo::main(args),
            "ar" | "cc" | "cxx" | "env" => env::main(args),
            "exec" => exec::main(args),
            "pkg" => pkg::main(args),
            "toolchain" => toolchain::main(args),
            _ => usage(),
        },
        None => usage(),
    }
}
