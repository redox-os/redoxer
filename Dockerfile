FROM ubuntu:24.04

# Install dependencies
RUN export DEBIAN_FRONTEND=noninteractive && \
    apt-get update -qq && \
    apt-get install -y -qq --no-install-recommends \
      build-essential \
      ca-certificates \
      curl \
      expect \
      fuse \
      libfuse-dev \
      pkg-config \
      qemu-system-x86 \
      rsync \
      nasm \
    && rm -rf /var/lib/apt/lists/*

# Install rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
    sh -s -- --default-toolchain nightly --profile minimal -y

# Set path
ENV PATH=/root/.cargo/bin:$PATH

# Install redoxer
COPY . /root/redoxer
RUN cargo install --path /root/redoxer && \
    rm -rf /root/redoxer /root/.cargo

# Install redoxer toolchain
RUN TARGET=x86_64-unknown-redox redoxer toolchain

# Ensure redoxer exec is working
RUN redoxer exec true
