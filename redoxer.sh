#!/usr/bin/env bash

set -ex

if [ ! -f "$1" ]
then
    echo "$0 [program]" >&2
    exit 1
fi

export TARGET=x86_64-unknown-redox

sudo rm -rf build/redoxer build/redoxer.bin
rm -f build/redoxer-qemu.bin
mkdir -p build/redoxer
if ! which redox_installer >/dev/null
then
    cargo install --git https://gitlab.redox-os.org/redox-os/installer.git
fi
sudo "$(which redox_installer)" -c redoxer.toml build/redoxer

name="$(basename "$1")"
sudo cp "$1" "build/redoxer/bin/$name"
echo "stdio debug:" | sudo tee build/redoxer/etc/init.d/10_redoxer
echo "echo <redoxer>" | sudo tee -a build/redoxer/etc/init.d/10_redoxer
echo "$name" | sudo tee -a build/redoxer/etc/init.d/10_redoxer
echo "echo </redoxer>" | sudo tee -a build/redoxer/etc/init.d/10_redoxer
if ! which redoxfs >/dev/null
then
    cargo install redoxfs
fi
sudo "$(which redoxfs-ar)" build/redoxer.bin build/redoxer build/redoxer/bootloader

cp build/redoxer.bin build/redoxer-qemu.bin
qemu-system-x86_64 \
    -smp 4 \
    -m 2048 \
    -serial mon:stdio \
    -machine q35 \
    -device ich9-intel-hda -device hda-duplex \
    -netdev user,id=net0 -device e1000,netdev=net0 \
    -nographic -vga none \
    -enable-kvm \
    -cpu host \
    -drive file=build/redoxer-qemu.bin,format=raw
