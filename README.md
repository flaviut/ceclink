# CECLink RP2350 firmware

Rust firmware for a Seeed XIAO RP2350 inline HDMI CEC adapter. The build produces a UF2 and ELF using pinned Nix and Cargo dependencies.

## Build

```sh
nix develop
cargo build --locked --profile release-with-debug
picotool uf2 convert target/thumbv8m.main-none-eabihf/release-with-debug/ceclink -t elf ceclink.uf2
```

If direnv is enabled in your shell, run `direnv allow` once in this directory to enter the flake dev shell automatically.

For a reproducible production artifact:

```sh
nix build .#firmware
ls result/ceclink.{uf2,elf}
```

Format the Nix and Rust sources with `nix fmt`.

## Flash

Hold the XIAO RP2350's BOOT button while connecting its USB cable, then release the button to enter BOOTSEL mode. From the dev shell, flash the UF2 built above and reboot into the firmware:

```sh
picotool load -v -x ceclink.uf2
```

If you used `nix build .#firmware`, flash `result/ceclink.uf2` instead.

The normal USB connection does not provide JTAG or SWD. The USB reset interface described below can enter BOOTSEL for fwupd; the BOOT button remains available for manual flashing.

## fwupd on NixOS

The firmware exposes Raspberry Pi's USB reset interface alongside CDC ACM. fwupd's existing `rp-pico` plugin uses that interface to enter RP2350 BOOTSEL, then its `uf2` plugin writes the UF2 image. The overlay adds device matching for this adapter and the RP2350 ROM's `2e8a:000f` USB identity.

In a NixOS flake, include this repository as an input named `ceclink` and add `inputs.ceclink.nixosModules.fwupd` to the host's module list. Alternatively, add `inputs.ceclink.overlays.default` to `nixpkgs.overlays` and enable `services.fwupd.enable = true;`. Rebuild the NixOS configuration, then flash a firmware containing the USB reset interface once using the manual method above. `fwupdmgr get-devices` should then show the adapter as updatable. A signed or local fwupd CAB containing the UF2 and release metadata is still required for `fwupdmgr update` to offer an update.

The runtime quirk selects fwupd's `rp-pico` plugin by VID:PID; that plugin also checks for the USB reset interface, which ordinary Pulse-Eight adapters lack. Increment the firmware's USB `device_release` for future firmware versions so fwupd can report the installed version. The RP2350 ROM BOOTSEL button remains a recovery path.

## DDC diagnostics

The device exposes a second USB CDC ACM port named `DDC debug`, separate from the Pulse-Eight CEC port. On Linux, the persistent `/dev/serial/by-id/usb-CECLink_RP2350_HDMI_CEC_Adapter_<chip-id>-if00` link identifies the CEC port and the matching `-if02` link identifies the debug port. Find it with `ls /dev/serial/by-id/*RP2350_HDMI_CEC_Adapter*-if02`, then open that path with `cat`. The `DDC debug` interface label is visible in USB descriptors but is not included in the by-id link name. While the port is open, it sends one tab-separated `key=value` line per second. The first record is sent immediately. Fields have stable names and decimal integer values; `version=1` identifies the format. `sda` and `scl` are 0 or 1, `trace_0` and `trace_1` are raw PIO words, and `physical_address=65535` means unknown. The counters are cumulative since boot. For example:

```text
version=1\tuptime_ms=1234\tsda=1\tscl=1\tring_used=0\tring_peak=4\tstarts=2\tstops=2\twords=12\tfifo_stalls=0\tring_overflows=0\ttrace_len=2\ttrace_0=0\ttrace_1=0\tedid_write_addresses=1\tedid_read_addresses=1\tedid_bytes=8\tinvalid_words=0\tunacknowledged_addresses=0\tpio_pc=7\trx_empty=1\trx_full=0\tphysical_address=4096
```

The separators in the actual output are tab characters. The debug port requires no host command and remains available while the Linux CEC driver owns the other serial port.

The onboard RGB LED uses GPIO22 for data and GPIO23 for power. Blue means the firmware is running but has not captured a DDC START. Amber means DDC traffic was captured but no physical address was found. Red means a FIFO stall or SRAM ring overflow occurred. Green means the physical address was found. The serial diagnostics give the exact counters and GPIO levels.

For an active EDID read from the HDMI connector, use `nix run nixpkgs#ddcutil -- --edid-read-size=256 --disable-try-get-edid-from-sysfs detect` while the adapter is in the HDMI path. Compare counter values before and after the read.

## Connections

| XIAO pin | RP2350 GPIO | Role |
| --- | ---: | --- |
| D9 | GPIO4 | Bidirectional CEC, open drain |
| D4 | GPIO6 | DDC SDA input only |
| D5 | GPIO7 | DDC SCL input only |

D4 and D5 are receive-only GPIO inputs, not I²C master pins. Use a 100kΩ resistor between the HDMI bus and the RP2350.

## Firmware structure

- [`src/main.rs`](src/main.rs): RP2350 clocks, pins, two USB CDC interfaces, and core startup.
- [`src/debug_port.rs`](src/debug_port.rs): periodic diagnostics on the second USB serial port.
- [`src/cec.rs`](src/cec.rs): CEC timing engine on core 1.
- [`src/ddc.rs`](src/ddc.rs): passive PIO capture and an interrupt that copies I²C records into a 512-word SRAM ring.
- [`src/ddc_protocol.rs`](src/ddc_protocol.rs): foreground I²C/EDID decoding and CTA HDMI VSDB physical address extraction.
- [`src/pulse8.rs`](src/pulse8.rs): Pulse-Eight serial framing and Linux driver command subset.
- [`src/transport.rs`](src/transport.rs): short critical sections for cross-core messages.

The USB device uses Pulse-Eight VID:PID `2548:1002`, as specified in `details.txt`, and reports `CECLink` / `RP2350 HDMI CEC Adapter` as its USB manufacturer and product strings. Its USB serial number is the RP2350's 64-bit chip ID in hexadecimal; if the ID cannot be read, the serial descriptor is omitted. On Linux, attach the in-tree driver to the `Pulse-Eight CEC control` interface with `inputattach --pulse8-cec /dev/ttyACM0`; the TTY number may differ. The `DDC debug` interface is a separate TTY and must not be passed to `inputattach`. Setting the serial line discipline may require root privileges. The kernel CEC device should then appear as `/dev/cec*`. If an autoattach udev rule matches the manufacturer or product strings, update it to match the CEC interface or use the VID:PID and interface number together.

The DDC sniffer only learns an address when it sees the host read the relevant EDID extension. A PIO FIFO stall or SRAM ring overflow discards the incomplete capture and waits for a new I²C START. DDC timing and capture still need hardware testing.
