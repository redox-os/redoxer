#!/usr/bin/env bash

set -e

export TARGET=x86_64-unknown-redox
xargo rustc --bin redoxerd --release --target "${TARGET}" -- -C linker="${TARGET}-gcc"

if [ ! -n "$1" ]
then
    echo "$0 [program] <args...>" >&2
    exit 1
fi

if ! which redox_installer >/dev/null
then
    echo "redox_installer not found" >&2
    cargo \
        install \
        --git https://gitlab.redox-os.org/redox-os/installer.git
fi

if ! which redoxfs >/dev/null
then
    echo "redoxfs not found" >&2
    cargo \
        install \
        redoxfs
fi

if ! which qemu-system-x86_64 >/dev/null
then
    echo "qemu-system-x86_64 not found" >&2
    exit 1
fi


function redoxfs_mounted {
    if [ -z "$1" ]
    then
        echo "redoxfs_mounted [directory]" >&2
        return 1
    fi

    # TODO: Escape path
    grep "^/dev/fuse $(realpath -m "$1") fuse" /proc/mounts >/dev/null
}

function redoxfs_unmount {
    if [ -z "$1" ]
    then
        echo "redoxfs_unmount [directory]" >&2
        return 1
    fi

    if redoxfs_mounted "$1"
    then
        fusermount -u "$1"
    fi

    if ! redoxfs_mounted "$1"
    then
        return 0
    else
        echo "redoxfs_unmount: failed to unmount '$1'" >&2
        return 1
    fi
}

function redoxfs_mount {
    if [ -z "$1" -o -z "$2" ]
    then
        echo "redoxfs_mount [image] [directory]" >&2
        return 1
    fi

    if ! redoxfs_unmount "$2"
    then
        echo "redoxfs_mount: failed to first unmount '$2'" >&2
        return 1
    fi

    redoxfs "$1" "$2"
    while ! redoxfs_mounted "$2"
    do
        if ! pgrep redoxfs >/dev/null
        then
            echo "redoxfs_mount: failed to mount '$1' to '$2'" >&2
            return 1
        fi
    done
}

if [ ! -f build/bootloader.bin ]
then
    echo "building bootloader" >&2

    rm -rf build/bootloader.bin build/bootloader

    mkdir -p build/bootloader
    redox_installer -c bootloader.toml build/bootloader

    mv build/bootloader/bootloader build/bootloader.bin
fi

if [ ! -f build/base.bin ]
then
    echo "building base" >&2

    redoxfs_unmount build/base
    rm -rf build/base.bin build/base.bin.partial build/base

    dd if=/dev/zero of=build/base.bin.partial bs=4096 count=65536
    redoxfs-mkfs build/base.bin.partial build/bootloader.bin

    mkdir -p build/base
    redoxfs_mount build/base.bin.partial build/base

    redox_installer -c base.toml build/base

    redoxfs_unmount build/base

    mv build/base.bin.partial build/base.bin
fi

uuid="$(uuidgen)"
function cleanup {
    redoxfs_unmount "build/${uuid}"
    rm -rf "build/${uuid}" "build/${uuid}.bin" "build/${uuid}.log"
}
trap cleanup EXIT

cleanup

cp build/base.bin "build/${uuid}.bin"

mkdir -p "build/${uuid}"
redoxfs_mount "build/${uuid}.bin" "build/${uuid}"

cp "target/${TARGET}/release/redoxerd" "build/${uuid}/bin/redoxerd"

if [[ "$1" == */* ]]
then
    name="$(basename "$1")"
    cp "$1" "build/${uuid}/bin/${name}"
    echo "${name}" >> "build/${uuid}/etc/redoxerd"
    shift
fi

for arg in "$@"
do
    echo "${arg}" >> "build/${uuid}/etc/redoxerd"
done


redoxfs_unmount "build/${uuid}"

set +e

qemu-system-x86_64 \
    -enable-kvm \
    -cpu host \
    -machine q35 \
    -m 2048 \
    -smp 4 \
    -serial mon:stdio \
    -chardev file,id=log,path="build/${uuid}.log" \
    -device isa-debugcon,chardev=log \
    -device isa-debug-exit \
    -netdev user,id=net0 \
    -device e1000,netdev=net0 \
    -nographic \
    -vga none \
    -drive file="build/${uuid}.bin",format=raw
status="$(("$?" / 2))"

echo
echo "## redoxer (${status}) ##"
cat "build/${uuid}.log"

exit "${status}"
