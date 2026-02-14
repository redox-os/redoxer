use std::{collections::HashMap, env, ffi, process};

use anyhow::{anyhow, Context};

use crate::{gnu_target, host_target, status_error, target, toolchain};

pub fn command<S: AsRef<ffi::OsStr>>(program: S) -> anyhow::Result<process::Command> {
    let toolchain_dir = toolchain().context("unable to init toolchain")?;

    // PATH must be set first so cargo is sourced from the toolchain path
    {
        let path = env::var_os("PATH").unwrap_or(ffi::OsString::new());
        let mut paths = env::split_paths(&path).collect::<Vec<_>>();
        paths.insert(0, toolchain_dir.join("bin"));
        let new_path = env::join_paths(paths)?;
        unsafe {
            env::set_var("PATH", new_path);
        }
    }

    // CC
    let target = target();
    let gnu_target = gnu_target();
    let gnu_targets = generate_gnu_targets();
    let cc_target_var = target.replace("-", "_");
    let cargo_target_var = cc_target_var.to_uppercase();
    #[cfg(feature = "cli-pkg")]
    let is_cc = program.as_ref() != "env" && program.as_ref() != "cargo";

    let mut command = process::Command::new(program);
    for (k, v) in gnu_targets.iter() {
        if *k == "CC" || *k == "CXX" {
            if let Ok(cc_wrapper) = std::env::var("CC_WRAPPER") {
                if cc_wrapper.len() > 0 {
                    command.env(k, format!("{cc_wrapper} {v}"));
                    continue;
                }
            }
        }
        command.env(k, v);
        command.env(format!("{k}_{cc_target_var}"), &v);
    }

    // CARGO
    command.env(
        format!("CARGO_TARGET_{}_LINKER", cargo_target_var),
        gnu_targets.get("CC").unwrap(),
    );
    command.env("RUSTUP_TOOLCHAIN", &toolchain_dir);
    command.env("TARGET", target);
    command.env("GNU_TARGET", gnu_target);

    // RUSTFLAGS, TODO:
    // 1. we're setting global RUSTFLAGS to per-target RUSTFLAGS
    //      without a way to let user leave global RUSTFLAGS
    //      but that probably is ok, because no usecase to it
    // 2. Global RUSTFLAGS is really confusing because of this issue
    //      https://github.com/rust-lang/cargo/issues/4423
    //      which claims there's no RUSTFLAGS for build.rs
    // 3. There are no CARGO_TARGET_xxx_ENCODED_RUSTFLAGS
    let mut rustflags = env::var("RUSTFLAGS").unwrap_or("".to_string());

    // CPPFLAGS
    let mut cppflags = env::var("CPPFLAGS").unwrap_or_else(|_| "".to_string());
    if !cppflags.is_empty() {
        cppflags += " ";
    }
    match target {
        "aarch64-unknown-redox" => cppflags += "-mno-outline-atomics",
        "riscv64gc-unknown-redox" => cppflags += "-march=rv64gc -mabi=lp64d",
        _ => {}
    }

    #[cfg(feature = "cli-pkg")]
    if let Some(sysroot) = crate::pkg::get_sysroot() {
        // pkg-config crate specific
        command.env(
            format!("PKG_CONFIG_PATH_{}", cc_target_var),
            sysroot.join("lib/pkgconfig"),
        );
        command.env(
            format!("PKG_CONFIG_SYSROOT_DIR_{}", cc_target_var),
            &sysroot,
        );
        rustflags = format!(
            "{} -L native={}",
            rustflags,
            sysroot.join("lib").canonicalize()?.display()
        );

        if is_cc {
            let includedir = sysroot.join("include").canonicalize()?;
            cppflags += &format!(" -I{}", includedir.display());

            let libdir = sysroot.join("lib").canonicalize()?;
            let mut ldflags = format!(
                "-Wl,-rpath-link,{} -L{}",
                libdir.display(),
                libdir.display()
            );

            if let Ok(user_ldflags) = env::var("LDFLAGS") {
                ldflags = format!("{} {}", ldflags, user_ldflags);
            }

            command.env("LDFLAGS", ldflags);
        }
    }

    if !cppflags.is_empty() {
        command.env("CPPFLAGS", &cppflags);
        command.env(format!("CFLAGS_{}", cc_target_var), &cppflags);
        command.env(format!("CXXFLAGS_{}", cc_target_var), &cppflags);
    }
    if !rustflags.is_empty() {
        command.env(
            format!("CARGO_TARGET_{}_RUSTFLAGS", cargo_target_var),
            rustflags,
        );
        command.env_remove("RUSTFLAGS");
    }

    Ok(command)
}

fn inner<I: Iterator<Item = String>>(program: &str, args: I) -> anyhow::Result<()> {
    let program = match program {
        "env" => "env".to_string(),
        "ar" => format!("{}-ar", gnu_target()),
        "cc" => format!("{}-gcc", gnu_target()),
        "cxx" => format!("{}-g++", gnu_target()),
        _ => return Err(anyhow!("Unknown env program {:?}", program)),
    };
    command(program)?
        .args(args)
        .status()
        .and_then(status_error)?;

    Ok(())
}

fn generate_gnu_targets() -> HashMap<&'static str, String> {
    let is_host = host_target() == target();
    let target_prefix = if is_host {
        "".to_string()
    } else {
        format!("{}-", gnu_target())
    };
    let mut h = HashMap::new();
    h.insert("AR", format!("{}gcc-ar", target_prefix));
    h.insert("AS", format!("{}as", target_prefix));
    h.insert("CC", format!("{}gcc", target_prefix));
    h.insert("CXX", format!("{}g++", target_prefix));
    h.insert("LD", format!("{}ld", target_prefix));
    h.insert("NM", format!("{}gcc-nm", target_prefix));
    h.insert("OBJCOPY", format!("{}objcopy", target_prefix));
    h.insert("OBJDUMP", format!("{}objdump", target_prefix));
    h.insert("PKG_CONFIG", format!("{}pkg-config", target_prefix));
    h.insert("RANLIB", format!("{}gcc-ranlib", target_prefix));
    h.insert("READELF", format!("{}readelf", target_prefix));
    h.insert("STRIP", format!("{}strip", target_prefix));

    if is_host {
        for (k, v) in h.iter_mut() {
            if let Some(env) = std::env::var(format!("REDOXER_HOST_{}", k)).ok() {
                if env.len() > 0 {
                    *v = env;
                }
            }
        }
    }
    h
}

pub fn main(args: &[String]) {
    match inner(args.get(1).unwrap(), args.iter().cloned().skip(2)) {
        Ok(()) => {
            process::exit(0);
        }
        Err(err) => {
            eprintln!("redoxer env: {:#}", err);
            process::exit(1);
        }
    }
}
