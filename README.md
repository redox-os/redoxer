# redoxer

The tool used to build/run Rust programs (and C/C++ programs with zero dependencies) inside of a Redox VM, the Redox GitLab CI use a Docker image with `redoxer` pre-installed.

A pre-built Docker image can be found on [Docker Hub](https://hub.docker.com/r/redoxos/redoxer)

## Options

```
redoxer <bench | build | check | doc | install | run | rustc | test>
    Run as cargo passed by `redoxer env cargo`
    Additionally set `redoxer exec` as test runner

redoxer env <command> [arguments]...
    Run as command with env configured to run with the toolchain
    The toolchain will be initialized by `redoxer toolchain`

redoxer exec [-f|--folder folder] [-g|--gui] [-h|--help] [-o|--output file] [--] <command> [arguments]...
    Run a command inside QEMU, using a redox image
    The redox image will be initialized if not exist
    Environment flags:
        REDOXER_QEMU_BINARY   Override qemu binary
        REDOXER_QEMU_ARGS     Override qemu args
        REDOXER_USE_FUSE      [true|false] Override use fuse (default is automatically detected)

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
