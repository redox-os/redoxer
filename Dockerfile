FROM ubuntu:18.04

# Install dependencies
RUN apt-get update -qq && \
    apt-get install -y -qq \
      build-essential \
      curl \
      libfuse-dev \
      pkg-config

# Install rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
    sh -s -- --default-toolchain nightly -y

# Set path
ENV PATH=/root/.cargo/bin:$PATH

# Install redoxer
COPY . /root/redoxer
RUN cargo install --path /root/redoxer

# Install redoxer toolchain
RUN redoxer install

# Install qemu-system-x86_64 (for redoxer exec)
RUN apt-get install -y -qq qemu-system-x86

# Install redoxfs (for redoxer exec)
RUN cargo install redoxfs
