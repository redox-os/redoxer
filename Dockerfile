FROM ubuntu:20.04

# Install dependencies
RUN export DEBIAN_FRONTEND=noninteractive && \
    apt-get update -qq && \
    apt-get install -y -qq \
      build-essential \
      curl \
      fuse \
      libfuse-dev \
      pkg-config \
      qemu-system-x86

# Install rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
    sh -s -- --default-toolchain nightly -y

# Set path
ENV PATH=/root/.cargo/bin:$PATH

# Install redoxer
COPY . /root/redoxer
RUN cargo install --path /root/redoxer

# Install redoxfs
RUN cargo install redoxfs

# Install redoxer toolchain
RUN redoxer toolchain

# Run test application
RUN redoxer exec true
