# Surface Laptop 7 Release Checklist

Target hardware:
- Microsoft Surface Laptop 7 (15")
- Snapdragon X Elite
- 16 GB RAM / 512 GB SSD
- Windows 11 dual-boot present

Build requirements:
- Build ARM64 ISO from the current `master` branch.
- Verify checksums (`.sha256` preferred, `.md5` optional).

Pre-boot checks:
1. Confirm the ISO was built from a clean git tree (only intended release changes).
2. Confirm `config/bootloaders/grub-pc/grub.cfg` contains:
   - `OxyOS Live (Surface Laptop 7 15")`
   - `OxyOS Live (Surface Laptop 7 15" safe graphics)`
3. Confirm `build.sh` sets ARM64 cmdline with `clk_ignore_unused pd_ignore_unused cma=128M`.
4. Confirm `config/hooks/live/0910-arm64-snapdragon.hook.chroot` is executable (`100755`) and does not force-load `msm` in initramfs or `snapdragon.conf`.
5. Confirm `config/package-lists/arm64-firmware.list.chroot` includes:
   - `dislocker`
   - `flash-kernel`
   - `protection-domain-mapper`

Boot validation on device:
1. Boot USB and select `OxyOS Live (Surface Laptop 7 15")`.
2. If black screen/reboot occurs, reboot and select `OxyOS Live (Surface Laptop 7 15" safe graphics)`.
3. In live session, run:
   - `sudo oxyos-qcom-firmware`
4. Verify no immediate reboot after firmware extraction and driver reload.
5. Reboot and test normal entry again.

Functional validation:
1. Display reaches desktop (no black screen loop).
2. Keyboard and touchpad work.
3. Wi-Fi network scan works.
4. System remains stable for at least 10 minutes idle + light interaction.
5. Optional: run `dmesg -T | grep -Ei "panic|oops|watchdog|msm|adreno"` and review for critical errors.

Release gate:
- Do not publish the ARM64 ISO until all boot and functional checks above pass on real hardware.
