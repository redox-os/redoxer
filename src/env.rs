use std::{collections::HashMap, env, ffi, process};

use anyhow::{anyhow, Context};

use crate::{gnu_target, host_target, status_error, target, toolchain};

fn append_flag(buf: &mut String, flag: &'static str) {
    if !buf.is_empty() {
        buf.push(' ');
    }
    buf.push_str(flag);
}

fn append_flag2(buf: &mut String, flag: &'static str, flag2: &str) {
    if !buf.is_empty() {
        buf.push(' ');
    }
    buf.push_str(flag);
    // TODO: Quote spaces
    buf.push_str(flag2);
}

pub fn command<S: AsRef<ffi::OsStr>>(program: S) -> anyhow::Result<process::Command> {
    let toolchain_dir = toolchain().context("unable to init toolchain")?;

    // PATH must be set first so cargo is sourced from the toolchain path
    {
        let path = env::var_os("PATH").unwrap_or_default();
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
    let is_clang = crate::is_use_clang();
    let mut command = process::Command::new(program);
    for (k, v) in gnu_targets.iter() {
        if *k == "CC" || *k == "CXX" {
            if let Ok(cc_wrapper) = std::env::var("CC_WRAPPER") {
                if !cc_wrapper.is_empty() {
                    command.env(k, format!("{cc_wrapper} {v}"));
                    continue;
                }
            }
        }
        command.env(k, v);
        command.env(format!("{k}_{cc_target_var}"), v);
    }

    // CARGO
    command.env(
        format!("CARGO_TARGET_{}_LINKER", cargo_target_var),
        if is_clang {
            "clang"
        } else {
            gnu_targets.get("CC").unwrap()
        },
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

    if is_clang && host_target() != target {
        // add args from cc
        let cc_args = gnu_targets.get("CC").unwrap().split(' ').skip(1);
        for arg in cc_args {
            append_flag2(&mut rustflags, "-C link-arg=", arg);
        }
    }

    // CPPFLAGS
    let mut cppflags = env::var("CPPFLAGS").unwrap_or_else(|_| "".to_string());
    match target {
        "aarch64-unknown-redox" => append_flag(&mut cppflags, "-mno-outline-atomics"),
        "riscv64gc-unknown-redox" => append_flag(&mut cppflags, "-march=rv64gc -mabi=lp64d"),
        _ => {}
    }

    // LDFLAGS
    let mut ldflags = env::var("LDFLAGS").unwrap_or("".to_string());
    // TODO: https://gitlab.redox-os.org/redox-os/redox/-/issues/1788
    // if is_clang {
    //     append_flag(&mut ldflags, "-fuse-ld=lld");
    // }

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

        let libdir = sysroot.join("lib").canonicalize()?;
        if let Some(libdir) = libdir.to_str() {
            append_flag(&mut rustflags, "-L");
            append_flag2(&mut rustflags, "native=", libdir);
        }

        if is_cc {
            let includedir = sysroot.join("include").canonicalize()?;
            if let Some(includedir) = includedir.to_str() {
                append_flag2(&mut cppflags, "-I", includedir);
            }
            if let Some(libdir) = libdir.to_str() {
                append_flag2(&mut ldflags, "-Wl,-rpath-link,", libdir);
                append_flag2(&mut ldflags, "-L", libdir);
            }
        }
    }

    if !cppflags.is_empty() {
        command.env("CPPFLAGS", &cppflags);
        command.env(format!("CFLAGS_{}", cc_target_var), &cppflags);
        command.env(format!("CXXFLAGS_{}", cc_target_var), &cppflags);
    }
    if !ldflags.is_empty() {
        command.env("LDFLAGS", ldflags);
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
    let clang = crate::is_use_clang();
    let program = match program {
        "env" => "env".to_string(),
        "ar" if clang => "llvm-ar".to_string(),
        "cc" if clang => "clang".to_string(),
        "cxx" if clang => "clang++".to_string(),
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
    let mut h = HashMap::new();
    if !crate::is_use_clang() {
        let target_prefix = if is_host {
            "".to_string()
        } else {
            format!("{}-", gnu_target())
        };

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
    } else {
        let target_flag = if is_host {
            "".to_string()
        } else {
            let toolchain = toolchain()
                .expect("Should have toolchain init")
                .join(gnu_target());
            // TODO: define __redox__ in clang
            format!(
                " --target={} --sysroot={} -D__redox__",
                gnu_target(),
                toolchain.display()
            )
        };

        h.insert("AR", "llvm-ar".to_string());
        h.insert("LD", "ld.lld".to_string());
        h.insert("NM", "llvm-nm".to_string());
        h.insert("OBJCOPY", "llvm-objcopy".to_string());
        h.insert("OBJDUMP", "llvm-objdump".to_string());
        h.insert("RANLIB", "llvm-ranlib".to_string());
        h.insert("READELF", "llvm-readelf".to_string());
        h.insert("STRIP", "llvm-strip".to_string());
        h.insert("AS", format!("clang{}", target_flag));
        h.insert("CC", format!("clang{}", target_flag));
        h.insert("CXX", format!("clang++{}", target_flag));
        h.insert("PKG_CONFIG", "pkg-config".to_string());
    }
    if is_host {
        for (k, v) in h.iter_mut() {
            if let Ok(env) = std::env::var(format!("REDOXER_HOST_{}", k)) {
                if !env.is_empty() {
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
