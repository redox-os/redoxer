# redoxer

The tool used to build/run Rust programs (and C/C++ programs with zero dependencies) inside of a Redox VM, the Redox GitLab CI use a Docker image with `redoxer` pre-installed.

A pre-built Docker image can be found on [Docker Hub](https://hub.docker.com/r/redoxos/redoxer)

## Options

```
redoxer env <command> [arguments]...
    Run as command with env configured to run with the toolchain
    The toolchain will be initialized by `redoxer toolchain`
    Environment flags:
        REDOXER_SYSROOT      Specify sysroot to link (default is target/$TARGET/sysroot on Cargo projects)

redoxer <bench | build | check | doc | fetch | install | run | rustc | test> [-g|--gui] [-o|--output file] [--] [arguments]
    Run as cargo passed by `redoxer env cargo`
    Additionally set `redoxer exec` as test runner

redoxer <ar | cc | cxx> [arguments]
    Run as GNU compiler passed by `redoxer env $GNU_TARGET-*`

redoxer exec [-f|--folder folder] [-f|--folder folder:/path/in/redox] [-g|--gui] [-h|--help] [-i|--install-config] [-o|--output file] [--] <command> [arguments]...
    Run a command inside QEMU, using a "base" or "gui" redox image, or provide custom one with --install-config
    The redox image will be initialized if not exist or different with the specified --install-config
    Specify a folder to copy it into /root inside redox image, or more generic one with folder:path
    If folder for /root is not specified but <command> is a file, the file will be copied
    Environment flags:
        REDOXER_QEMU_BINARY   Override qemu binary
        REDOXER_QEMU_ARGS     Override qemu args
        REDOXER_USE_FUSE      [true|false] Override use fuse (default is automatically detected)

redoxer pkg [install|remove|update] pkg-1 pkg-2 ...
    Install additional native packages for Cargo
    Environment flags:
        REDOXER_SYSROOT     Where to install sysroot (default is target/$TARGET/sysroot on Cargo projects)
        REDOXER_PKG_SOURCE  Override source of packages (default is https://static.redox-os.org/pkg)

redoxer toolchain [--update] [--url PATH]
    Install or manage toolchain
    Environment flags:
        REDOXER_TOOLCHAIN   Override toolchain path
```

## Commands

- Install the tool

```sh
cargo install redoxer
```

- Install the Redox toolchain

```sh
redoxer toolchain
```

- Update the Redox toolchain using prebuilt toolchain from existing Redox OS Repo

```sh
make prefix/x86_64-unknown-redox/relibc-install.tar.gz
redoxer toolchain --update --url .
```

- Build the Rust program or library with Redoxer

```sh
redoxer build
```

- Build the Rust program or library with additional native packages

```sh
redoxer pkg install xz
redoxer build
```


- Run the Rust program on Redox

```sh
redoxer run
```

- Test the Rust program or library with Redoxer

```sh
redoxer test
```

- Run arbitrary executable (`echo hello`) with Redoxer

```sh
redoxer exec echo hello
```

## Host specific customizations

`redoxer env` can be configured to compile host binaries by setting `TARGET` to the correct host target:

+ `*-unknown-redox`
+ `*-unknown-linux-gnu`
+ `*-unknown-linux-musl`
+ `*-unknown-freebsd`
+ `*-apple-darwin`

This feature is mainly used for [Redox build system](https://gitlab.redox-os.org/redox-os/redox). For other than Linux and Redox, you must supply additional environments to the correct paths or binary name for GCC and Binutils. You can also use it to specify other compiler or different version of GCC. Here's an example for using the system default:

```sh
export REDOXER_HOST_AR=ar
export REDOXER_HOST_AS=as
export REDOXER_HOST_CC=cc
export REDOXER_HOST_CXX=c++
export REDOXER_HOST_LD=ld
export REDOXER_HOST_NM=nm
export REDOXER_HOST_OBJCOPY=objcopy
export REDOXER_HOST_OBJDUMP=objdump
export REDOXER_HOST_PKG_CONFIG=pkg-config
export REDOXER_HOST_RANLIB=ranlib
export REDOXER_HOST_READELF=readelf
export REDOXER_HOST_STRIP=strip
```

