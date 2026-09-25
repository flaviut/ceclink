"""Apply CECLink status records to the Linux CEC adapter on the same USB device."""

import argparse
import array
import fcntl
import logging
import os
import time
from pathlib import Path


# Linux CEC UAPI ioctl numbers on the supported x86_64 and aarch64 systems.
CEC_ADAP_G_PHYS_ADDR = 0x80026101
CEC_ADAP_S_PHYS_ADDR = 0x40026102


def parse_status(line):
    try:
        fields = dict(field.split("=", 1) for field in line.decode("ascii").strip().split("\t"))
        if set(fields) != {"status_version", "physical_address"}:
            return None
        if fields["status_version"] != "1":
            return None
        address = int(fields["physical_address"])
    except (UnicodeError, ValueError):
        return None
    if not 0 <= address <= 0xFFFF:
        return None
    if address != 0xFFFF:
        zero = False
        for shift in (12, 8, 4, 0):
            nibble = (address >> shift) & 15
            if zero and nibble:
                return None
            zero |= nibble == 0
    return address


def usb_parent(path):
    path = path.resolve(strict=True)
    for parent in (path, *path.parents):
        if (parent / "idVendor").exists() and (parent / "idProduct").exists():
            return parent
    raise OSError(f"No USB parent for {path}")


def cec_sysfs_path(device, sysfs):
    number = device.stat().st_rdev
    return sysfs / "dev/char" / f"{os.major(number)}:{os.minor(number)}"


def matching_cec(tty, sysfs=Path("/sys"), devices=Path("/dev")):
    usb = usb_parent(sysfs / "class/tty" / tty)
    matches = []
    for device in devices.glob("cec*"):
        if not device.name[3:].isdigit():
            continue
        try:
            if usb_parent(cec_sysfs_path(device, sysfs)) == usb:
                matches.append(device)
        except OSError:
            continue
    # Never guess if a driver has not attached or multiple adapters match.
    return matches[0] if len(matches) == 1 else None


def apply_address(device, address):
    with device.open("rb+", buffering=0) as port:
        current = array.array("H", [0])
        fcntl.ioctl(port, CEC_ADAP_G_PHYS_ADDR, current, True)
        if current[0] == address:
            return False
        fcntl.ioctl(port, CEC_ADAP_S_PHYS_ADDR, array.array("H", [address]))
    return True


def synchronize(tty, address):
    device = matching_cec(tty)
    if device is not None and apply_address(device, address):
        logging.info("%s physical address: %04x", device, address)


class AddressFilter:
    """Let EDID rereads finish before publishing a topology change."""

    def __init__(self):
        self.candidate = None
        self.since = 0.0

    def settled(self, address, now):
        if address != self.candidate:
            self.candidate = address
            self.since = now
        delay = 2.0 if address == 0xFFFF else 0.5
        return now - self.since >= delay


def main():
    import serial

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tty", help="CECLink status TTY name, e.g. ttyACM1")
    args = parser.parse_args()
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    address_filter = AddressFilter()
    # pyserial asserts DTR so the device emits status records.
    with serial.Serial(f"/dev/{args.tty}", timeout=2, exclusive=True) as port:
        while True:
            line = port.read_until(b"\n", size=1024)
            if not line.endswith(b"\n"):
                continue
            address = parse_status(line)
            if address is None:
                continue
            if not address_filter.settled(address, time.monotonic()):
                continue
            try:
                synchronize(args.tty, address)
            except OSError as error:
                # The next periodic record retries device enumeration and
                # address writes, even if the physical address is unchanged.
                logging.warning("Physical address update deferred: %s", error)


if __name__ == "__main__":
    main()
