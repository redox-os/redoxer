FROM ubuntu:18.04

# Install dependencies
RUN apt-get update -qq && \
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

# Install redoxfs
RUN cargo install redoxfs

# Install redoxer
COPY . /root/redoxer
RUN cargo install --path /root/redoxer

# Install redoxer toolchain
RUN redoxer install
