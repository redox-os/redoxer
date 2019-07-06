use std::{env, io, path, process};

pub (crate) use self::toolchain::toolchain;

mod cargo;
mod exec;
mod install;
mod redoxfs;
mod toolchain;

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

//TODO: Confirm capabilities on other OSes
#[cfg(target_os = "linux")]
fn running(program: &str) -> io::Result<bool> {
    process::Command::new("pgrep")
        .arg(program)
        .stdout(process::Stdio::null())
        .status()
        .map(|x| x.success())
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
    eprintln!("redoxer exec - execute a command in Redox VM");
    eprintln!("redoxer install - install toolchain");
    eprintln!("redoxer run - cargo run with Redox target in Redox VM");
    eprintln!("redoxer rustc - cargo rustc with Redox target");
    eprintln!("redoxer test - cargo test with Redox target in Redox VM");
    process::exit(1);
}

fn main() {
    match env::args().nth(1) {
        Some(arg) => match arg.as_str() {
            "bench" |
            "build" |
            "check" |
            "doc" |
            "run" |
            "rustc" |
            "test" => cargo::main(),
            "exec" => exec::main(),
            "install" => install::main(),
            _ => usage(),
        },
        None => usage(),
    }
}
