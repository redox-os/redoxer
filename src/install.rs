use std::{io, process};

use crate::toolchain;

fn inner() -> io::Result<()> {
    let _toolchain_dir = toolchain()?;

    Ok(())
}

pub fn main() {
    match inner() {
        Ok(()) => {
            process::exit(0);
        },
        Err(err) => {
            eprintln!("redoxer install: {}", err);
            process::exit(1);
        }
    }
}
