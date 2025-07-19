# redoxer

The tool used to build/run Rust programs (and C/C++ programs with zero dependencies) inside of a Redox VM, the Redox GitLab CI use a Docker image with `redoxer` pre-installed.

A pre-built Docker image can be found on [Docker Hub](https://hub.docker.com/r/redoxos/redoxer)

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
