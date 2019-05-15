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

fn usage() {
    eprintln!("redoxer build - build with cargo");
    eprintln!("redoxer exec - execute a command");
    eprintln!("redoxer install - install toolchain");
    process::exit(1);
}

fn main() {
    match env::args().nth(1) {
        Some(arg) => match arg.as_str() {
            "build" => cargo::main(),
            "exec" => exec::main(),
            "install" => install::main(),
            "run" => cargo::main(),
            "test" => cargo::main(),
            _ => usage(),
        },
        None => usage(),
    }
}
