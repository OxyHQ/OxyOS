#!/bin/bash
set -e

# OxyOS Multi-Architecture Build Script
# Usage: ./build.sh [--arch amd64|arm64] [--clean]

ARCH="amd64"
CLEAN=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --arch)
            ARCH="$2"
            shift 2
            ;;
        --clean)
            CLEAN=true
            shift
            ;;
        -h|--help)
            echo "Usage: ./build.sh [--arch amd64|arm64] [--clean]"
            echo ""
            echo "Options:"
            echo "  --arch   Target architecture: amd64 (default) or arm64"
            echo "  --clean  Run lb clean before building"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

if [[ "$ARCH" != "amd64" && "$ARCH" != "arm64" ]]; then
    echo "Error: Architecture must be 'amd64' or 'arm64'"
    exit 1
fi

echo "========================================="
echo "  OxyOS Build - Architecture: $ARCH"
echo "========================================="

ARM64_PKGLIST="config/package-lists/arm64-firmware.list.chroot"
ARM64_PKGLIST_DISABLED="${ARM64_PKGLIST}.disabled"

# --- Configure architecture-specific settings ---

# Bootstrap
sed -i "s|^LB_ARCHITECTURE=.*|LB_ARCHITECTURE=\"${ARCH}\"|" config/bootstrap

# Chroot - kernel flavour
sed -i "s|^LB_LINUX_FLAVOURS_WITH_ARCH=.*|LB_LINUX_FLAVOURS_WITH_ARCH=\"${ARCH}\"|" config/chroot

if [[ "$ARCH" == "arm64" ]]; then
    # ARM64: EFI-only boot (no BIOS/syslinux)
    sed -i 's|^LB_BOOTLOADER_BIOS=.*|LB_BOOTLOADER_BIOS=""|' config/binary
    sed -i 's|^LB_BOOTAPPEND_LIVE=.*|LB_BOOTAPPEND_LIVE="boot=live components quiet splash username=live clk_ignore_unused pd_ignore_unused"|' config/binary
    sed -i 's|^LB_BOOTAPPEND_LIVE_FAILSAFE=.*|LB_BOOTAPPEND_LIVE_FAILSAFE="boot=live components memtest nomodeset nosplash clk_ignore_unused pd_ignore_unused"|' config/binary

    # Enable ARM64 firmware package list
    if [ -f "$ARM64_PKGLIST_DISABLED" ]; then
        mv "$ARM64_PKGLIST_DISABLED" "$ARM64_PKGLIST"
    fi

    # Cross-build support: if building on amd64 host, configure qemu
    HOST_ARCH=$(dpkg --print-architecture 2>/dev/null || uname -m)
    if [[ "$HOST_ARCH" == "amd64" || "$HOST_ARCH" == "x86_64" ]]; then
        echo "Detected x86_64 host - configuring cross-build with qemu-aarch64-static"
        sed -i 's|^LB_BOOTSTRAP_QEMU_ARCHITECTURE=.*|LB_BOOTSTRAP_QEMU_ARCHITECTURE="arm64"|' config/bootstrap
        sed -i 's|^LB_BOOTSTRAP_QEMU_EXCLUDE=.*|LB_BOOTSTRAP_QEMU_EXCLUDE=""|' config/bootstrap
        sed -i 's|^LB_BOOTSTRAP_QEMU_STATIC=.*|LB_BOOTSTRAP_QEMU_STATIC="/usr/bin/qemu-aarch64-static"|' config/bootstrap
    fi

    echo "[ARM64] Bootloader: grub-efi only (no syslinux)"
    echo "[ARM64] Boot params: clk_ignore_unused pd_ignore_unused"
    echo "[ARM64] Firmware packages: enabled"
    echo "[ARM64] Kernel: will be upgraded from Debian Sid via 0905-arm64-kernel hook"
else
    # AMD64: restore defaults
    sed -i 's|^LB_BOOTLOADER_BIOS=.*|LB_BOOTLOADER_BIOS="syslinux"|' config/binary
    sed -i 's|^LB_BOOTAPPEND_LIVE=.*|LB_BOOTAPPEND_LIVE="boot=live components quiet splash username=live"|' config/binary
    sed -i 's|^LB_BOOTAPPEND_LIVE_FAILSAFE=.*|LB_BOOTAPPEND_LIVE_FAILSAFE="boot=live components memtest noapic noapm nodma nomce nolapic nomodeset nosmp nosplash vga=788"|' config/binary

    # Disable ARM64 firmware package list for amd64 builds
    if [ -f "$ARM64_PKGLIST" ]; then
        mv "$ARM64_PKGLIST" "$ARM64_PKGLIST_DISABLED"
    fi

    # Clear cross-build settings
    sed -i 's|^LB_BOOTSTRAP_QEMU_ARCHITECTURE=.*|LB_BOOTSTRAP_QEMU_ARCHITECTURE=""|' config/bootstrap
    sed -i 's|^LB_BOOTSTRAP_QEMU_EXCLUDE=.*|LB_BOOTSTRAP_QEMU_EXCLUDE=""|' config/bootstrap
    sed -i 's|^LB_BOOTSTRAP_QEMU_STATIC=.*|LB_BOOTSTRAP_QEMU_STATIC=""|' config/bootstrap
fi

echo "Configuration updated for ${ARCH}"

# --- Build ---

if $CLEAN; then
    echo "Cleaning previous build..."
    lb clean
fi

mkdir -p .build && touch .build/config
lb build

# Rename output ISO with architecture suffix
OUTPUT_ISO=$(ls -1 live-image-${ARCH}.hybrid.iso 2>/dev/null || true)
if [ -n "$OUTPUT_ISO" ]; then
    FINAL_NAME="oxyos-1.0-${ARCH}.iso"
    mv "$OUTPUT_ISO" "$FINAL_NAME"
    echo "Output: ${FINAL_NAME}"
fi

echo "========================================="
echo "  Build complete: ${ARCH}"
echo "========================================="
