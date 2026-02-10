## OxyOS
# OxyOS - A lightweight desktop Linux project.

OxyOS is a lightweight, elegant desktop Linux distribution built on Debian.

Over the years, under-the-hood updates have included systemd transitions, display/login stack changes, and GTK updates while keeping a fast-and-minimal experience as intact as possible.

Official website: https://os.oxy.so/

Lastly, we'd like to thank Philip for all his hard work through the years, the legacy he's created, and the bar he's set for sleek high-performance distros.

### The ISO

In this repository, you'll find the sources used to generate the actual system image, with installer, along with some configurations.

### Codebase Layout

This repository is the live-build configuration for assembling OxyOS installation media.

- `config/package-lists/` - package selections installed into the live system.
- `config/includes.chroot_after_packages/` - files copied into the target root filesystem after package installation.
- `config/includes.installer/` - installer-time assets and configuration.
- `config/bootloaders/` - boot splash and bootloader-specific resources.
- `config/hooks/` - live-build hook scripts used during image creation.
- `config/preseed/` - Debian installer automation defaults.
- `config/archives/` - apt source list fragments used during build/install stages.

Build internals use `oxyos` identifiers throughout.

### Building

OxyOS supports building for both **amd64** (x86-64) and **arm64** (AArch64/Snapdragon X) architectures using the `build.sh` script.

#### Build with Docker (recommended)

**amd64 (default):**
```
$ docker run --privileged --cap-add=ALL -v /proc:/proc -v /sys:/sys -v $PWD:/build -w /build -it --rm debian:trixie /bin/sh -c 'apt-get update && apt-get install -y live-build && mkdir -p .build && touch .build/config && bash build.sh --arch amd64'
```

**arm64 (Snapdragon X / ARM laptops):**
```
$ docker run --privileged --cap-add=ALL -v /proc:/proc -v /sys:/sys -v $PWD:/build -w /build -it --rm debian:trixie /bin/sh -c 'apt-get update && apt-get install -y live-build qemu-user-static && mkdir -p .build && touch .build/config && bash build.sh --arch arm64'
```

#### Build without Docker

```
# apt-get update && apt-get install -y live-build && bash build.sh --arch amd64
# apt-get update && apt-get install -y live-build qemu-user-static && bash build.sh --arch arm64
```

#### ARM64 / Snapdragon X Notes

The ARM64 build includes Qualcomm firmware support for Snapdragon X Elite/Plus devices. After booting the live USB, most devices require firmware extraction from the Windows partition:

```
$ sudo oxyos-qcom-firmware
```

This extracts GPU and other firmware from the Windows NTFS partition. A notification will remind you on first boot.

### OxyOS Project Sources

The OxyOS project is split across several repositories. The current public source list is:

- [OxyOS-website](https://github.com/OxyHQ/OxyOS-website) - Source code for the website available at https://os.oxy.so/ (CSS, updated Oct 29, 2025)
- [OxyOS-menu](https://github.com/OxyHQ/OxyOS-menu) - Menu tooling for OxyOS (Python, updated Aug 22, 2025)
- [OxyOS-apps](https://github.com/OxyHQ/OxyOS-apps) - OxyOS apps package sources (Python, MIT License, updated Aug 22, 2025)
- [OxyOS-welcome](https://github.com/OxyHQ/OxyOS-welcome) - Optional post-installation script for OxyOS (Shell, updated Aug 22, 2025)
- [OxyOS-wallpapers](https://github.com/OxyHQ/OxyOS-wallpapers) - A selection of wallpapers (updated Aug 22, 2025)
- [OxyOS-ui-theme](https://github.com/OxyHQ/OxyOS-ui-theme) - OxyOS user interface theme assets (CSS, updated Aug 22, 2025)
- [OxyOS-pipemenus](https://github.com/OxyHQ/OxyOS-pipemenus) - Openbox pipemenus for OxyOS (Shell, updated Aug 22, 2025)
- [OxyOS-metapackage](https://github.com/OxyHQ/OxyOS-metapackage) - Basic metapackage for OxyOS (updated Aug 22, 2025)
- [OxyOS-lxdm-theme](https://github.com/OxyHQ/OxyOS-lxdm-theme) - OxyOS LXDM theme resources (CSS, updated Aug 22, 2025)
- [OxyOS-icon-theme](https://github.com/OxyHQ/OxyOS-icon-theme) - Default icon themes for OxyOS (updated Aug 22, 2025)
- [OxyOS-exit](https://github.com/OxyHQ/OxyOS-exit) - Script prompt for system power-state options (Python, updated Aug 22, 2025)
- [OxyOS-configs](https://github.com/OxyHQ/OxyOS-configs) - Various configuration files for use with OxyOS (Shell, updated Aug 22, 2025)
- [OxyOS](https://github.com/OxyHQ/OxyOS) - Main OxyOS build and integration repository (Shell, updated Aug 22, 2025)
