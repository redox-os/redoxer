use std::{io, path, process};

pub (crate) use self::toolchain::toolchain;

mod cargo;
mod env;
mod exec;
mod redoxfs;
mod toolchain;

const SUPPORTED_TARGETS: &'static [&'static str] = &[
    "x86_64-unknown-redox",
    "aarch64-unknown-redox",
    "i686-unknown-redox",
];

//TODO: Confirm capabilities on other OSes
#[cfg(target_os = "linux")]
fn installed(program: &str) -> io::Result<bool> {
    process::Command::new("which")
        .arg(program)
        .stdout(process::Stdio::null())
        .status()
        .map(|x| x.success())
}

fn redoxer_dir() -> path::PathBuf {
    dirs::home_dir().unwrap_or(path::PathBuf::from("."))
        .join(".redoxer")
}

fn status_error(status: process::ExitStatus) -> io::Result<()> {
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Other,
            format!("{}", status)
        ))
    }
}

fn syscall_error(err: syscall::Error) -> io::Error {
    io::Error::from_raw_os_error(err.errno)
}

fn usage() {
    eprintln!("redoxer bench - cargo bench with Redox target in Redox VM");
    eprintln!("redoxer build - cargo build with Redox target");
    eprintln!("redoxer check - cargo check with Redox target");
    eprintln!("redoxer doc - cargo doc with Redox target");
    eprintln!("redoxer env - execute a command in cross-compilation environment");
    eprintln!("redoxer exec - execute a command in Redox VM");
    eprintln!("redoxer install - cargo install with Redox target");
    eprintln!("redoxer run - cargo run with Redox target in Redox VM");
    eprintln!("redoxer rustc - cargo rustc with Redox target");
    eprintln!("redoxer test - cargo test with Redox target in Redox VM");
    eprintln!("redoxer toolchain - install toolchain");
    process::exit(1);
}

pub fn target() -> &'static str {
    let target_from_env = std::env::var("TARGET").unwrap_or("".to_string());

    let index = if SUPPORTED_TARGETS.contains(&&*target_from_env) == true {
        SUPPORTED_TARGETS.iter().position(|t| **t == target_from_env).unwrap().into()
    } else {
        0usize
    };

    SUPPORTED_TARGETS[index]
}

pub fn main(args: &[String]) {
    match args.get(1) {
        Some(arg) => match arg.as_str() {
            "bench" |
            "build" |
            "check" |
            "doc" |
            "install" |
            "run" |
            "rustc" |
            "test" => cargo::main(args),
            "env" => env::main(args),
            "exec" => exec::main(args),
            "toolchain" => toolchain::main(args),
            _ => usage(),
        },
        None => usage(),
    }
}
