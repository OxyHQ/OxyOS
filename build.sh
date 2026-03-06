#!/bin/bash
set -e

# OxyOS Multi-Architecture Build Script
# Usage: ./build.sh [--arch amd64|arm64] [--clean]

ARCH="amd64"
VERSION="1.0"
CLEAN=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --arch)
            ARCH="$2"
            shift 2
            ;;
        --version)
            VERSION="$2"
            shift 2
            ;;
        --clean)
            CLEAN=true
            shift
            ;;
        -h|--help)
            echo "Usage: ./build.sh [--arch amd64|arm64] [--version X.Y] [--clean]"
            echo ""
            echo "Options:"
            echo "  --arch      Target architecture: amd64 (default) or arm64"
            echo "  --version   Version number (default: 1.0)"
            echo "  --clean     Run lb clean before building"
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
    sed -i 's|^LB_BOOTAPPEND_LIVE=.*|LB_BOOTAPPEND_LIVE="boot=live components username=live clk_ignore_unused pd_ignore_unused earlycon=efifb console=tty0 loglevel=7"|' config/binary
    sed -i 's|^LB_BOOTAPPEND_LIVE_FAILSAFE=.*|LB_BOOTAPPEND_LIVE_FAILSAFE="boot=live components nomodeset nosplash clk_ignore_unused pd_ignore_unused earlycon=efifb console=tty0 loglevel=7"|' config/binary

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

    # --- Compile Snapdragon X Device Tree Blobs ---
    # DTBs must be compiled before lb build so they go into config/includes.binary/
    # which live-build copies to the ISO automatically. Compiling inside a chroot hook
    # doesn't work because binary_rootfs (squashfs) runs before binary_hooks.
    # Uses two-stage compilation (matching Linux kernel's scripts/Makefile.lib):
    # Stage 1: cpp preprocesses DTS → temp file (resolves #include, macros)
    # Stage 2: dtc compiles temp file → binary DTB
    echo "[ARM64] Compiling Snapdragon X DTBs..."
    apt-get install -y --no-install-recommends device-tree-compiler cpp git ca-certificates

    DTB_OUT="$(pwd)/config/includes.binary/live/dtbs/qcom"
    mkdir -p "$DTB_OUT"

    REPO_URL="https://github.com/dwhinham/kernel-surface-pro-11.git"
    DTB_BRANCH="wip/x1e80100-6.17-sp11"
    WORK_DIR="/tmp/dts-source"

    git clone --depth=1 --sparse --filter=blob:none \
        "$REPO_URL" -b "$DTB_BRANCH" "$WORK_DIR"
    cd "$WORK_DIR"
    git sparse-checkout set arch/arm64/boot/dts/qcom include/dt-bindings include/linux include/uapi

    COMPILED=0
    FAILED=0
    for dts in arch/arm64/boot/dts/qcom/x1e*.dts arch/arm64/boot/dts/qcom/x1p*.dts; do
        [ -f "$dts" ] || continue
        dtb_name=$(basename "${dts%.dts}.dtb")
        tmp_file="/tmp/${dtb_name%.dtb}.dts.tmp"
        echo "  Compiling: $dtb_name"

        # Stage 1: Preprocess — resolves #include and macros, writes to temp file
        if cpp -nostdinc -P \
            -Iinclude \
            -Iarch/arm64/boot/dts \
            -Iarch/arm64/boot/dts/qcom \
            -undef -D__DTS__ -x assembler-with-cpp \
            "$dts" -o "$tmp_file"; then

            # Stage 2: Compile preprocessed DTS → binary DTB
            if dtc -I dts -O dtb -b 0 \
                -i arch/arm64/boot/dts/qcom \
                -o "$DTB_OUT/$dtb_name" \
                "$tmp_file"; then
                COMPILED=$((COMPILED + 1))
            else
                echo "  FAILED (dtc): $dtb_name"
                FAILED=$((FAILED + 1))
            fi
        else
            echo "  FAILED (cpp): $dtb_name"
            FAILED=$((FAILED + 1))
        fi
        rm -f "$tmp_file"
    done

    cd "$OLDPWD"
    rm -rf "$WORK_DIR"
    echo "[ARM64] DTBs: $COMPILED compiled, $FAILED failed"
    ls "$DTB_OUT"/*.dtb 2>/dev/null || true
    if [ "$COMPILED" -eq 0 ]; then
        echo "Error: No DTBs compiled successfully"
        exit 1
    fi

    echo "[ARM64] Bootloader: grub-efi only (no syslinux)"
    echo "[ARM64] Boot params: clk_ignore_unused pd_ignore_unused"
    echo "[ARM64] Firmware packages: enabled"
    echo "[ARM64] Kernel: will be upgraded from Experimental via 0905-arm64-kernel hook"
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
    FINAL_NAME="oxyos-${VERSION}-${ARCH}.iso"
    mv "$OUTPUT_ISO" "$FINAL_NAME"
    echo "Output: ${FINAL_NAME}"
fi

echo "========================================="
echo "  Build complete: ${ARCH}"
echo "========================================="
