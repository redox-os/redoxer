# redoxer

The tool used to build/run Rust programs (and C/C++ programs with zero dependencies) inside of a Redox VM, the Redox GitLab CI use a Docker image with `redoxer` pre-installed.

A pre-built Docker image can be found on [Docker Hub](https://hub.docker.com/r/redoxos/redoxer)

## Commands

- `cargo install redoxer` - install `redoxer` tool.
- `redoxer toolchain` - install `redoxer` toolchain.
- `redoxer build` - build project with `redoxer`.
- `redoxer run` - run project with `redoxer`.
- `redoxer test` - test project with `redoxer`.
- `redoxer exec echo hello` - run arbitrary executable with `redoxer`.