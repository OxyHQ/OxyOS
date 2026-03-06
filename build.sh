#!/bin/bash
set -euo pipefail

# OxyOS Multi-Architecture Build Script
# Usage: ./build.sh [--arch amd64|arm64] [--version X.Y] [--clean]

usage() {
    cat << 'EOF'
Usage: ./build.sh [--arch amd64|arm64] [--version X.Y] [--clean]

Options:
  --arch      Target architecture: amd64 (default) or arm64
  --version   Version number (default: 1.0)
  --clean     Run lb clean before building
  -h, --help  Show this help message
EOF
}

require_command() {
    local cmd="$1"
    if ! command -v "$cmd" > /dev/null 2>&1; then
        echo "Error: Missing required command: $cmd"
        exit 1
    fi
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

ARCH="amd64"
VERSION="1.0"
CLEAN=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --arch)
            if [[ $# -lt 2 || -z "${2:-}" || "${2:0:1}" == "-" ]]; then
                echo "Error: --arch requires a value (amd64|arm64)"
                exit 1
            fi
            ARCH="$2"
            shift 2
            ;;
        --version)
            if [[ $# -lt 2 || -z "${2:-}" || "${2:0:1}" == "-" ]]; then
                echo "Error: --version requires a value"
                exit 1
            fi
            VERSION="$2"
            shift 2
            ;;
        --clean)
            CLEAN=true
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            usage
            exit 1
            ;;
    esac
done

if [[ "$ARCH" != "amd64" && "$ARCH" != "arm64" ]]; then
    echo "Error: Architecture must be 'amd64' or 'arm64'"
    exit 1
fi

for cmd in lb sed cp mv; do
    require_command "$cmd"
done

echo "========================================="
echo "  OxyOS Build - Architecture: $ARCH"
echo "========================================="

ARM64_PKGLIST="config/package-lists/arm64-firmware.list.chroot"
ARM64_PKGLIST_DISABLED="${ARM64_PKGLIST}.disabled"
CONFIG_FILES=(
    "config/bootstrap"
    "config/chroot"
    "config/binary"
)

for cfg in "${CONFIG_FILES[@]}"; do
    if [ ! -f "$cfg" ]; then
        echo "Error: Required config file missing: $cfg"
        exit 1
    fi
done

PKGLIST_STATE="unknown"
if [ -f "$ARM64_PKGLIST" ]; then
    PKGLIST_STATE="enabled"
elif [ -f "$ARM64_PKGLIST_DISABLED" ]; then
    PKGLIST_STATE="disabled"
else
    echo "Error: Neither $ARM64_PKGLIST nor $ARM64_PKGLIST_DISABLED exists"
    exit 1
fi

BACKUP_DIR="$(mktemp -d /tmp/oxyos-build-backup.XXXXXX)"
WORK_DIR=""

for cfg in "${CONFIG_FILES[@]}"; do
    mkdir -p "$BACKUP_DIR/$(dirname "$cfg")"
    cp -a "$cfg" "$BACKUP_DIR/$cfg"
done

cleanup() {
    local exit_code=$?
    trap - EXIT INT TERM

    if [ -n "$WORK_DIR" ] && [ -d "$WORK_DIR" ]; then
        rm -rf "$WORK_DIR"
    fi

    for cfg in "${CONFIG_FILES[@]}"; do
        if [ -f "$BACKUP_DIR/$cfg" ]; then
            cp -a "$BACKUP_DIR/$cfg" "$cfg"
        fi
    done

    case "$PKGLIST_STATE" in
        enabled)
            if [ -f "$ARM64_PKGLIST_DISABLED" ] && [ ! -f "$ARM64_PKGLIST" ]; then
                mv "$ARM64_PKGLIST_DISABLED" "$ARM64_PKGLIST"
            fi
            ;;
        disabled)
            if [ -f "$ARM64_PKGLIST" ] && [ ! -f "$ARM64_PKGLIST_DISABLED" ]; then
                mv "$ARM64_PKGLIST" "$ARM64_PKGLIST_DISABLED"
            fi
            ;;
    esac

    rm -rf "$BACKUP_DIR"
    exit "$exit_code"
}
trap cleanup EXIT INT TERM

# --- Configure architecture-specific settings ---

# Bootstrap
sed -i "s|^LB_ARCHITECTURE=.*|LB_ARCHITECTURE=\"${ARCH}\"|" config/bootstrap

# Chroot - kernel flavour
sed -i "s|^LB_LINUX_FLAVOURS_WITH_ARCH=.*|LB_LINUX_FLAVOURS_WITH_ARCH=\"${ARCH}\"|" config/chroot

if [[ "$ARCH" == "arm64" ]]; then
    # ARM64: EFI-only boot (no BIOS/syslinux)
    sed -i 's|^LB_BOOTLOADER_BIOS=.*|LB_BOOTLOADER_BIOS=""|' config/binary
    sed -i 's|^LB_BOOTAPPEND_LIVE=.*|LB_BOOTAPPEND_LIVE="boot=live components quiet splash username=live clk_ignore_unused pd_ignore_unused cma=128M"|' config/binary
    sed -i 's|^LB_BOOTAPPEND_LIVE_FAILSAFE=.*|LB_BOOTAPPEND_LIVE_FAILSAFE="boot=live components nomodeset nosplash clk_ignore_unused pd_ignore_unused cma=128M"|' config/binary

    # Enable ARM64 firmware package list
    if [ -f "$ARM64_PKGLIST_DISABLED" ]; then
        mv "$ARM64_PKGLIST_DISABLED" "$ARM64_PKGLIST"
    fi

    # Cross-build support: if building on amd64 host, configure qemu
    HOST_ARCH="$(dpkg --print-architecture 2>/dev/null || uname -m)"
    if [[ "$HOST_ARCH" == "amd64" || "$HOST_ARCH" == "x86_64" ]]; then
        if [ ! -x "/usr/bin/qemu-aarch64-static" ]; then
            echo "Error: /usr/bin/qemu-aarch64-static is missing. Install qemu-user-static first."
            exit 1
        fi
        echo "Detected x86_64 host - configuring cross-build with qemu-aarch64-static"
        sed -i 's|^LB_BOOTSTRAP_QEMU_ARCHITECTURE=.*|LB_BOOTSTRAP_QEMU_ARCHITECTURE="arm64"|' config/bootstrap
        sed -i 's|^LB_BOOTSTRAP_QEMU_EXCLUDE=.*|LB_BOOTSTRAP_QEMU_EXCLUDE=""|' config/bootstrap
        sed -i 's|^LB_BOOTSTRAP_QEMU_STATIC=.*|LB_BOOTSTRAP_QEMU_STATIC="/usr/bin/qemu-aarch64-static"|' config/bootstrap
    fi

    # Ensure tools required for DTB build are available.
    MISSING_TOOLS=()
    for cmd in dtc cpp git; do
        if ! command -v "$cmd" > /dev/null 2>&1; then
            MISSING_TOOLS+=("$cmd")
        fi
    done
    if [ "${#MISSING_TOOLS[@]}" -gt 0 ]; then
        if ! command -v apt-get > /dev/null 2>&1; then
            echo "Error: Missing required tools (${MISSING_TOOLS[*]}) and apt-get is unavailable."
            exit 1
        fi
        if [ "$(id -u)" -ne 0 ]; then
            echo "Error: Missing required tools (${MISSING_TOOLS[*]}). Re-run as root or pre-install them."
            exit 1
        fi
        apt-get update
        apt-get install -y --no-install-recommends device-tree-compiler cpp git ca-certificates
    fi

    # --- Compile Snapdragon X Device Tree Blobs ---
    # DTBs must be compiled before lb build so they go into config/includes.binary/
    # which live-build copies to the ISO automatically.
    # Uses two-stage compilation:
    # Stage 1: cpp preprocesses DTS -> temp file
    # Stage 2: dtc compiles temp file -> binary DTB
    echo "[ARM64] Compiling Snapdragon X DTBs..."

    DTB_OUT="${SCRIPT_DIR}/config/includes.binary/live/dtbs/qcom"
    mkdir -p "$DTB_OUT"
    rm -f "$DTB_OUT"/*.dtb

    REPO_URL="https://github.com/dwhinham/kernel-surface-pro-11.git"
    DTB_BRANCH="wip/x1e80100-6.17-sp11"
    WORK_DIR="$(mktemp -d /tmp/oxyos-dts-source.XXXXXX)"

    git clone --depth=1 --sparse --filter=blob:none \
        "$REPO_URL" -b "$DTB_BRANCH" "$WORK_DIR"
    cd "$WORK_DIR"
    git sparse-checkout set arch/arm64/boot/dts/qcom include/dt-bindings include/linux include/uapi

    COMPILED=0
    FAILED=0
    for dts in arch/arm64/boot/dts/qcom/x1e*.dts arch/arm64/boot/dts/qcom/x1p*.dts; do
        [ -f "$dts" ] || continue
        dtb_name="$(basename "${dts%.dts}.dtb")"
        tmp_file="/tmp/${dtb_name%.dtb}.dts.tmp"
        echo "  Compiling: $dtb_name"

        # Stage 1: preprocess DTS
        if cpp -nostdinc -P \
            -Iinclude \
            -Iarch/arm64/boot/dts \
            -Iarch/arm64/boot/dts/qcom \
            -undef -D__DTS__ -x assembler-with-cpp \
            "$dts" -o "$tmp_file"; then

            # Stage 2: compile to DTB
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

    cd "$SCRIPT_DIR"
    rm -rf "$WORK_DIR"
    WORK_DIR=""

    echo "[ARM64] DTBs: $COMPILED compiled, $FAILED failed"
    ls "$DTB_OUT"/*.dtb 2>/dev/null || true

    if [ "$COMPILED" -eq 0 ]; then
        echo "Error: No DTBs compiled successfully"
        exit 1
    fi

    if [ ! -f "$DTB_OUT/x1e80100-microsoft-romulus15.dtb" ]; then
        echo "Warning: Surface Laptop 7 15\" DTB missing; boot will fall back to generic CRD DTB."
    fi

    echo "[ARM64] Bootloader: grub-efi only (no syslinux)"
    echo "[ARM64] Boot params: clk_ignore_unused pd_ignore_unused cma=128M"
    echo "[ARM64] Firmware packages: enabled"
    echo "[ARM64] Kernel: upgraded via 0905-arm64-kernel hook"
else
    # AMD64 defaults
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

mkdir -p .build
touch .build/config
lb build

# Rename output ISO with architecture suffix
OUTPUT_ISO="live-image-${ARCH}.hybrid.iso"
if [ -f "$OUTPUT_ISO" ]; then
    FINAL_NAME="oxyos-${VERSION}-${ARCH}.iso"
    mv "$OUTPUT_ISO" "$FINAL_NAME"
    echo "Output: ${FINAL_NAME}"
else
    echo "Warning: Expected ISO not found at ${OUTPUT_ISO}"
fi

echo "========================================="
echo "  Build complete: ${ARCH}"
echo "========================================="
