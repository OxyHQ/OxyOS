"""A QEMU machine booting an image: QMP for the screen and the keyboard, the serial port as a
console. Standard library only."""

import json
import os
import pathlib
import shutil
import socket
import subprocess
import tempfile
import threading
import time

# QEMU's key codes (QKeyCode) for what the tests type at a bootloader and a shell.
KEYS = {" ": "spc", "-": "minus", "=": "equal", ",": "comma", ".": "dot", "/": "slash", ";": "semicolon",
        "'": "apostrophe", "\n": "ret", "\t": "tab", "\\": "backslash", "[": "bracket_left", "]": "bracket_right"}
SHIFTED = {"_": "minus", "+": "equal", ":": "semicolon", '"': "apostrophe", "|": "backslash", "<": "comma",
           ">": "dot", "?": "slash", "!": "1", "@": "2", "#": "3", "$": "4", "%": "5", "^": "6", "&": "7",
           "*": "8", "(": "9", ")": "0", "{": "bracket_left", "}": "bracket_right"}

FIRMWARE = {
    # architecture: {name: (code, variables template, Secure Boot)}
    # amd64: Debian's ovmf. The Secure Boot variables hold Microsoft's keys, which Debian's shim
    # is signed with.
    "amd64": {
        "uefi": ("/usr/share/OVMF/OVMF_CODE_4M.fd", "/usr/share/OVMF/OVMF_VARS_4M.fd", False),
        "secureboot": ("/usr/share/OVMF/OVMF_CODE_4M.secboot.fd", "/usr/share/OVMF/OVMF_VARS_4M.ms.fd", True),
        "bios": (None, None, False),
    },
    # arm64: Debian's qemu-efi-aarch64 (AAVMF, EDK2 for QEMU's virt machine). UEFI only.
    "arm64": {
        "uefi": ("/usr/share/AAVMF/AAVMF_CODE.no-secboot.fd", "/usr/share/AAVMF/AAVMF_VARS.fd", False),
        "secureboot": ("/usr/share/AAVMF/AAVMF_CODE.secboot.fd", "/usr/share/AAVMF/AAVMF_VARS.ms.fd", True),
    },
}

# The serial port each machine has, which the kernel names: the PC's 16550 (ttyS0), the virt
# machine's PL011 (ttyAMA0).
SERIAL_CONSOLE = {"amd64": "ttyS0", "arm64": "ttyAMA0"}

# The firmware each package provides, for the error that names what is missing.
FIRMWARE_PACKAGE = {"amd64": "ovmf", "arm64": "qemu-efi-aarch64"}


class Failure(Exception):
    pass


def default_accel(architecture):
    """KVM when the machine can run the image's architecture natively, else TCG."""
    native = {"x86_64": "amd64", "aarch64": "arm64"}.get(os.uname().machine)
    return "kvm" if native == architecture and os.access("/dev/kvm", os.R_OK | os.W_OK) else "tcg"


class Machine:
    """One boot of a machine. `image` is the medium in its optical drive (None: no medium). A
    machine that boots more than once (an install, then the installed system) keeps its `disk` and
    its firmware's variables (`variables`, copied from the firmware's template the first time), as
    a computer keeps its disk and NVRAM. `smbios` are the firmware's OEM strings (SMBIOS type 11):
    systemd-boot appends `io.systemd.boot.kernel-cmdline-extra=` to every entry's command line,
    the way a virtual machine is given a console without changing the image. `debug_serial` adds a
    second serial port (ttyS1), for systemd's debug shell; `usb_keyboard` adds a USB keyboard, which
    OVMF reads in systemd-boot's menu (its PS/2 driver gives systemd-boot no keys). `external` is a
    second disk, never booted from: an external disk connected to the computer (a backup disk)."""

    def __init__(self, image, directory, firmware="uefi", resolution=(1920, 1200), display="std",
                 accel=None, memory=2048, cpus=None, architecture="amd64", disk=None, variables=None,
                 smbios=(), debug_serial=False, usb_keyboard=False, external=None):
        self.image = pathlib.Path(image) if image else None
        self.disk = pathlib.Path(disk) if disk else None
        self.external = pathlib.Path(external) if external else None
        self.variables = pathlib.Path(variables) if variables else None
        self.smbios, self.debug_serial, self.usb_keyboard = list(smbios), debug_serial, usb_keyboard
        self.debug_log = bytearray()
        self.directory = pathlib.Path(directory)
        self.directory.mkdir(parents=True, exist_ok=True)
        self.architecture = architecture
        if firmware not in FIRMWARE[architecture]:
            raise Failure(f"no {firmware} firmware on {architecture} (it boots {', '.join(FIRMWARE[architecture])})")
        self.firmware, self.resolution, self.display = firmware, resolution, display
        self.accel = accel or default_accel(architecture)
        # Emulated (TCG), more CPUs make up for each one's slowness: QEMU runs one thread each.
        self.memory, self.cpus = memory, cpus or (2 if self.accel == "kvm" else 4)
        self.process = None
        self.serial_log = bytearray()
        self.serial_lock = threading.Lock()
        self.started = None
        self._id = 0

    # The machine.

    def command_line(self):
        width, height = self.resolution
        arm = self.architecture == "arm64"
        if arm:
            # QEMU's virt machine, as AAVMF expects; the newest GIC the accelerator offers. With
            # TCG, pointer authentication with QEMU's own algorithm: the architected one (QARMA)
            # costs more to emulate than the rest of the CPU.
            machine = "virt,gic-version=max,accel=" + self.accel
            cpu = "host" if self.accel == "kvm" else "max,pauth-impdef=on"
            arguments = ["qemu-system-aarch64"]
        else:
            machine = "q35,accel=" + self.accel
            cpu = "host" if self.accel == "kvm" else "max"
            arguments = ["qemu-system-x86_64"]
        # A machine with a disk restarts as a computer does (an install, an erase); a medium alone
        # is booted once, and its end is QEMU's.
        arguments += ["-nodefaults", "-display", "none"] + ([] if self.disk else ["-no-reboot"]) + [
                      "-m", str(self.memory), "-smp", str(self.cpus), "-cpu", cpu]
        code, variables, secure = FIRMWARE[self.architecture][self.firmware]
        if code:
            if not pathlib.Path(code).exists():
                raise Failure(f"no {self.firmware} firmware at {code} (Debian's {FIRMWARE_PACKAGE[self.architecture]} package)")
            copy = self.variables or self.directory / "VARS.fd"
            if not (self.variables and copy.exists()):
                shutil.copyfile(variables, copy)
            if secure and not arm:
                # OVMF keeps its variables out of the OS's reach in SMM; the virt machine has no
                # SMM, and AAVMF's Secure Boot needs none.
                machine += ",smm=on"
                arguments += ["-global", "driver=cfi.pflash01,property=secure,value=on"]
            arguments += ["-drive", f"if=pflash,format=raw,unit=0,readonly=on,file={code}",
                          "-drive", f"if=pflash,format=raw,unit=1,file={copy}"]
        arguments += ["-machine", machine]
        # A display whose preferred mode (its EDID) is the size under test.
        if arm:
            # The virt machine has no VGA: virtio-gpu, which AAVMF draws its console on and the
            # kernel's virtio-gpu driver takes over.
            arguments += ["-device", f"virtio-gpu-pci,edid=on,xres={width},yres={height}"]
        elif self.display == "virtio":
            arguments += ["-device", f"virtio-vga,edid=on,xres={width},yres={height}"]
        else:
            # bochs-drm offers only modes that fit its video memory twice: 16 MB drops 1920x1200.
            arguments += ["-device", f"VGA,edid=on,xres={width},yres={height},vgamem_mb=64"]
        if self.image:
            arguments += ["-drive", f"file={self.image},media=cdrom,readonly=on,if=none,id=cd"]
        if arm:
            # No IDE or PS/2 on virt: the medium on virtio-scsi, a USB keyboard (the one AAVMF's
            # menu and GRUB read, where QMP's keys arrive).
            arguments += ["-device", "virtio-scsi-pci,id=scsi", "-device", "qemu-xhci,id=usb", "-device", "usb-kbd,bus=usb.0"]
            if self.image:
                arguments += ["-device", "scsi-cd,bus=scsi.0,drive=cd,bootindex=0"]
        else:
            if self.image:
                arguments += ["-device", "ide-cd,drive=cd,bootindex=0"]
            if self.usb_keyboard:
                arguments += ["-device", "qemu-xhci,id=usb", "-device", "usb-kbd,bus=usb.0"]
        if self.disk:
            # A disk that can be discarded, as an SSD is (an erase discards the old root).
            arguments += ["-drive", f"file={self.disk},format=raw,if=none,id=disk,discard=unmap",
                          "-device", "virtio-blk-pci,drive=disk,bootindex=1"]
        if self.external:
            # After the disk, so it is the second (vdb), and with no boot index.
            arguments += ["-drive", f"file={self.external},format=raw,if=none,id=external",
                          "-device", "virtio-blk-pci,drive=external"]
        for value in self.smbios:
            # QEMU's option syntax doubles a literal comma.
            arguments += ["-smbios", "type=11,value=" + value.replace(",", ",,")]
        arguments += [
            "-nic", "user,model=virtio-net-pci",
            "-device", "virtio-rng-pci",
            "-qmp", f"unix:{self.sockets / 'qmp.sock'},server=on,wait=off",
            "-chardev", f"socket,id=serial,path={self.sockets / 'serial.sock'},server=on,wait=off",
            "-serial", "chardev:serial",
        ]
        if self.debug_serial:
            arguments += ["-chardev", f"socket,id=debug,path={self.sockets / 'debug.sock'},server=on,wait=off",
                          "-serial", "chardev:debug"]
        return arguments

    def start(self):
        # Unix sockets' paths are limited to 107 bytes, which a run's directory may exceed: they are
        # in a short directory of their own, removed when the machine stops.
        self.sockets = pathlib.Path(tempfile.mkdtemp(prefix="oxyos-qemu-"))
        self.process = subprocess.Popen(self.command_line(), stdout=subprocess.DEVNULL,
                                        stderr=open(self.directory / "qemu.log", "wb"))
        self.started = time.monotonic()
        self.qmp = self._connect("qmp.sock")
        self.qmp_file = self.qmp.makefile("rwb")
        greeting = json.loads(self.qmp_file.readline())
        if "QMP" not in greeting:
            raise Failure(f"unexpected QMP greeting {greeting}")
        self.execute("qmp_capabilities")
        self.serial = self._connect("serial.sock")
        threading.Thread(target=self._read_serial, daemon=True).start()
        if self.debug_serial:
            self.debug = self._connect("debug.sock")
            threading.Thread(target=self._read_debug, daemon=True).start()
        return self

    def elapsed(self):
        return time.monotonic() - self.started

    def _connect(self, name):
        path = str(self.sockets / name)
        deadline = time.monotonic() + 30
        while True:
            try:
                connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                connection.connect(path)
                return connection
            except OSError as error:
                connection.close()
                if self.process.poll() is not None:
                    raise Failure(f"QEMU exited ({self.process.returncode}); see {self.directory / 'qemu.log'}") from error
                if time.monotonic() > deadline:
                    raise
                time.sleep(0.05)

    def stop(self):
        if self.process and self.process.poll() is None:
            try:
                self.execute("quit")
            except Exception:
                pass
            try:
                self.process.wait(10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        (self.directory / "serial.log").write_bytes(bytes(self.serial_log))
        if self.debug_serial:
            (self.directory / "debug.log").write_bytes(bytes(self.debug_log))
        shutil.rmtree(self.sockets, ignore_errors=True)
        for name in ("VARS.fd", "screen.ppm"):
            (self.directory / name).unlink(missing_ok=True)

    # QMP.

    def execute(self, command, **arguments):
        self._id += 1
        self.qmp_file.write(json.dumps({"execute": command, "arguments": arguments, "id": self._id}).encode() + b"\n")
        self.qmp_file.flush()
        while True:
            reply = json.loads(self.qmp_file.readline())
            if reply.get("id") != self._id:
                continue  # an event
            if "error" in reply:
                raise Failure(f"QMP {command}: {reply['error']}")
            return reply.get("return")

    def screendump(self, path):
        self.execute("screendump", filename=str(path), format="ppm")

    def keys(self, *names, hold=50):
        self.execute("send-key", keys=[{"type": "qcode", "data": name} for name in names], **{"hold-time": hold})
        # Bootloaders poll the keyboard: a key sent before the last is released is lost.
        time.sleep(hold / 1000 + 0.07)

    def type(self, text):
        for character in text:
            if character.isalpha() and character.isupper():
                self.keys("shift", character.lower())
            elif character.isalnum():
                self.keys(character)
            elif character in KEYS:
                self.keys(KEYS[character])
            elif character in SHIFTED:
                self.keys("shift", SHIFTED[character])
            else:
                raise ValueError(f"no key for {character!r}")

    # The serial console.

    def _read_serial(self):
        while True:
            try:
                data = self.serial.recv(4096)
            except OSError:
                return
            if not data:
                return
            with self.serial_lock:
                self.serial_log += data

    def _read_debug(self):
        while True:
            try:
                data = self.debug.recv(4096)
            except OSError:
                return
            if not data:
                return
            with self.serial_lock:
                self.debug_log += data

    def debug_text(self):
        with self.serial_lock:
            return self.debug_log.decode("utf-8", errors="replace")

    def exited(self):
        return self.process.poll() is not None

    def serial_text(self):
        with self.serial_lock:
            return self.serial_log.decode("utf-8", errors="replace")

    def wait_serial(self, *needles, timeout, after=0):
        """Wait for any of `needles` in the serial output after offset `after`; return
        (needle, offset just past it)."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            text = self.serial_text()
            for needle in needles:
                index = text.find(needle, after)
                if index >= 0:
                    return needle, index + len(needle)
            if self.process.poll() is not None:
                raise Failure(f"QEMU exited ({self.process.returncode}) waiting for {needles}")
            time.sleep(0.1)
        raise Failure(f"no {' or '.join(map(repr, needles))} on the serial console within {timeout} s")

    def send_serial(self, text):
        self.serial.sendall(text.encode())

    def run(self, command, timeout=120, debug=False):
        """Run `command` in the serial console's shell (`debug`: the debug shell's, on ttyS1) and
        return its output."""
        begin, end = "@@OXYOS-BEGIN@@", "@@OXYOS-END@@"
        text = self.debug_text if debug else self.serial_text
        send = self.debug.sendall if debug else self.serial.sendall
        # The markers are split in what is typed, so the echoed command never matches them.
        mark = len(text())
        send(f"printf '%s\\n' '@@OXYOS-BEGIN''@@'; {command}; printf '%s\\n' '@@OXYOS-END''@@'\r".encode())
        _, start = self.wait_text(text, (begin + "\r\n", begin + "\n"), timeout, mark)
        _, stop = self.wait_text(text, (end,), timeout, start)
        return text()[start:stop - len(end)].replace("\r", "").strip("\n")

    def wait_text(self, text, needles, timeout, after=0):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            current = text()
            for needle in needles:
                index = current.find(needle, after)
                if index >= 0:
                    return needle, index + len(needle)
            if self.process.poll() is not None:
                raise Failure(f"QEMU exited ({self.process.returncode}) waiting for {needles}")
            time.sleep(0.1)
        raise Failure(f"no {' or '.join(map(repr, needles))} within {timeout} s")
