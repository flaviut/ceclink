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

## NixOS integration

Add this repository as a flake input and import `inputs.ceclink.nixosModules.default` on the host. The module loads the Pulse-Eight kernel driver, starts `inputattach` only for the CEC USB interface, creates `/dev/ceclink-status` for the status interface, and enables fwupd support.

The `ceclink-physical-address@` service reads the status interface and applies its DDC physical address to the matching `/dev/cec*` through the standard Linux CEC ioctl. It matches the devices by their shared USB parent, including when more than one adapter is connected. It retries when the CEC driver is still attaching and rechecks the kernel's address on every record. If firmware reports `65535` (unknown) for 2 seconds, the helper uses `ddcutil detect` to force a 256-byte EDID read across display buses, up to three attempts, while the firmware is listening. The firmware status confirms whether a read found the address. Known addresses must stay stable for 0.5 seconds; a short unknown interval during an EDID reread does not start another scan or clear a valid address. The NixOS module loads `i2c-dev` and supplies `ddcutil`. On other Linux distributions, install `pyserial` and `ddcutil`, load `i2c-dev`, and run `python3 nix/physical-address.py ttyACM1`, substituting the status TTY. The service needs access to the status TTY, the CEC device, and the HDMI I²C buses.

## fwupd on NixOS

The firmware exposes Raspberry Pi's USB reset interface alongside CDC ACM. fwupd's existing `rp-pico` plugin uses that interface to enter RP2350 BOOTSEL, then its `uf2` plugin writes the UF2 image. The overlay adds device matching for this adapter and the RP2350 ROM's `2e8a:000f` USB identity.

Rebuild the NixOS configuration, then flash a firmware containing the USB reset interface once using the manual method above. `fwupdmgr get-devices` should then show the adapter as updatable. A signed or local fwupd CAB containing the UF2 and release metadata is still required for `fwupdmgr update` to offer an update.

The runtime quirk selects fwupd's `rp-pico` plugin by VID:PID; that plugin also checks for the USB reset interface, which ordinary Pulse-Eight adapters lack. Increment the firmware's USB `device_release` for future firmware versions so fwupd can report the installed version. The RP2350 ROM BOOTSEL button remains a recovery path.

## Physical address status and DDC diagnostics

The second USB CDC ACM port is labeled `CECLink status`. On Linux, the persistent `/dev/serial/by-id/usb-CECLink_RP2350_HDMI_CEC_Adapter_<chip-id>-if00` link identifies the CEC port; `-if02` identifies status. While DTR is asserted, status sends a record immediately, when the address changes, and once per second. The stable format is exactly two tab-separated decimal fields; `65535` means unknown:

```text
status_version=1\tphysical_address=4096
```

The separator on the wire is a tab. For a verbose snapshot, send `diagnostics\n` to the status port. It returns one record with `version=1`, GPIO levels, PIO traces, counters, and the current address. This diagnostic record is separate from the supported status format. For example:

```text
version=1\tuptime_ms=1234\tsda=1\tscl=1\tring_used=0\tring_peak=4\tstarts=2\tstops=2\twords=12\tfifo_stalls=0\tring_overflows=0\ttrace_len=2\ttrace_0=0\ttrace_1=0\tedid_write_addresses=1\tedid_read_addresses=1\tedid_bytes=8\tinvalid_words=0\tunacknowledged_addresses=0\tpio_pc=7\trx_empty=1\trx_full=0\tphysical_address=4096
```

Stop `ceclink-physical-address@<status-tty>.service` before opening the status port directly. The CEC control port remains available to `inputattach` while the helper uses status.

For a future driver, the CEC control port also supports the CECLink `GET_SNIFFED_PHYSICAL_ADDRESS` command (`0x30`). Send the normal Pulse-Eight frame `ff 30 fe`; the response is `ff 30 <address-high> <address-low> fe`, using the normal escape rules for bytes `fd` through `ff`. This returns the raw DDC address, including `ffff` when unknown, regardless of any host-set physical address. The stock driver does not send this command.

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
- [`src/debug_port.rs`](src/debug_port.rs): stable status and requested diagnostics on the second USB serial port.
- [`src/cec.rs`](src/cec.rs): CEC timing engine on core 1.
- [`src/ddc.rs`](src/ddc.rs): passive PIO capture and an interrupt that copies I²C records into a 512-word SRAM ring.
- [`src/ddc_protocol.rs`](src/ddc_protocol.rs): foreground I²C/EDID decoding and CTA HDMI VSDB physical address extraction.
- [`src/pulse8.rs`](src/pulse8.rs): Pulse-Eight serial framing and Linux driver command subset.
- [`src/transport.rs`](src/transport.rs): short critical sections for cross-core messages.

The USB device uses Pulse-Eight VID:PID `2548:1002`, as specified in `details.txt`, and reports `CECLink` / `RP2350 HDMI CEC Adapter` as its USB manufacturer and product strings. Its USB serial number is the RP2350's 64-bit chip ID in hexadecimal; if the ID cannot be read, the serial descriptor is omitted. On Linux, attach the in-tree driver to the `Pulse-Eight CEC control` interface with `inputattach --pulse8-cec /dev/ttyACM0`; the TTY number may differ. The status interface is a separate TTY and must not be passed to `inputattach`. Setting the serial line discipline may require root privileges. The kernel CEC device should then appear as `/dev/cec*`. If an autoattach udev rule matches the manufacturer or product strings, update it to match the CEC interface or use the VID:PID and interface number together.

The DDC sniffer only learns an address when it sees the host read the relevant EDID extension. A PIO FIFO stall or SRAM ring overflow discards the incomplete capture and waits for a new I²C START. DDC timing and capture still need hardware testing.
