#!/usr/bin/env python3
"""Query RP2350 DDC capture diagnostics over its Pulse-Eight serial link.

The diagnostic command is firmware-specific; it is not part of Pulse-Eight's
protocol. Run without arguments to get one snapshot, --watch to watch activity,
or --bootsel to ask the firmware to enter the USB bootloader.
"""

import argparse
import glob
import os
import select
import termios
import time
import tty

START, END, ESCAPE = 0xFF, 0xFE, 0xFD


def default_port():
    matches = glob.glob(
        "/dev/serial/by-id/usb-CEC_4K_RP2350_HDMI_CEC_Adapter_*"
    )
    return matches[0] if len(matches) == 1 else "/dev/ttyACM0"


def frame(payload):
    result = bytearray([START])
    for value in payload:
        if value >= ESCAPE:
            result.extend((ESCAPE, value - 3))
        else:
            result.append(value)
    result.append(END)
    return result


def read_frame(fd, timeout=2.0):
    deadline = time.monotonic() + timeout
    started = escaped = False
    data = bytearray()
    while time.monotonic() < deadline:
        ready, _, _ = select.select([fd], [], [], max(0, min(0.2, deadline - time.monotonic())))
        if not ready:
            continue
        for value in os.read(fd, 256):
            if value == START and not escaped:
                started, escaped = True, False
                data.clear()
            elif started and value == ESCAPE and not escaped:
                escaped = True
            elif started and value == END and not escaped:
                return bytes(data)
            elif started:
                data.append((value + 3) & 0xFF if escaped else value)
                escaped = False
    raise TimeoutError("No complete reply from the adapter")


def request(fd, code):
    os.write(fd, frame(bytes([code])))
    deadline = time.monotonic() + 2.0
    while time.monotonic() < deadline:
        reply = read_frame(fd, max(0.1, deadline - time.monotonic()))
        if reply and reply[0] == code:
            return reply
    raise TimeoutError(f"No response to command 0x{code:02x}")


def word(data, start, size=4):
    return int.from_bytes(data[start : start + size], "big")


def describe_trace(raw):
    if raw & 0x7FFFFF:
        return f"0x{raw:08x} (not a nine-bit word)"
    bits = int(f"{raw:032b}"[::-1], 2) & 0x1FF
    return f"0x{raw:08x} (byte 0x{bits >> 1:02x}, {'ACK' if bits & 1 == 0 else 'NACK'})"


def show(data):
    if len(data) != 58 or data[0] != 0x40 or data[1] != 1:
        raise ValueError(f"Unexpected diagnostic reply: {data.hex(' ')}")
    data = data[1:]
    levels = data[1]
    print(f"GPIO6 SDA: {'high' if levels & 1 else 'low'}; GPIO7 SCL: {'high' if levels & 2 else 'low'}")
    print(f"SRAM ring: {word(data, 2, 2)} queued, peak {word(data, 4, 2)} / 512")
    for label, offset in [
        ("START events", 6),
        ("STOP events", 10),
        ("nine-bit words", 14),
        ("PIO FIFO stalls", 18),
        ("SRAM ring overflows", 22),
        ("EDID write addresses", 35),
        ("EDID read addresses", 39),
        ("EDID data bytes", 43),
        ("invalid words", 47),
        ("unacknowledged addresses", 51),
    ]:
        print(f"{label}: {word(data, offset)}")
    print(f"PIO program counter: {data[55]}; RX FIFO flags: 0x{data[56]:02x}")
    for index in range(min(data[26], 2)):
        print(f"word after last START #{index + 1}: {describe_trace(word(data, 27 + index * 4))}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", default=default_port())
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--watch", action="store_true", help="query once per second")
    mode.add_argument("--bootsel", action="store_true", help="reboot into BOOTSEL")
    args = parser.parse_args()

    fd = os.open(args.port, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
    old = termios.tcgetattr(fd)
    try:
        tty.setraw(fd)
        attrs = termios.tcgetattr(fd)
        attrs[4] = attrs[5] = termios.B115200
        termios.tcsetattr(fd, termios.TCSANOW, attrs)
        termios.tcflush(fd, termios.TCIFLUSH)
        if args.bootsel:
            os.write(fd, frame(b"\x41RP25"))
            print("Requested BOOTSEL; the USB serial device should disconnect.")
            return
        while True:
            show(request(fd, 0x40))
            address = request(fd, 0x1F)
            if len(address) == 3:
                print(f"Physical address: 0x{word(address, 1, 2):04x}")
            if not args.watch:
                break
            print()
            time.sleep(1)
    finally:
        try:
            termios.tcsetattr(fd, termios.TCSANOW, old)
        except termios.error:
            pass  # BOOTSEL disconnects the device.
        os.close(fd)


if __name__ == "__main__":
    main()
