# CEC 4K RP2350 firmware

Rust firmware for a Seeed XIAO RP2350 inline HDMI CEC adapter. The build produces a UF2 and ELF using pinned Nix and Cargo dependencies.

## Build

```sh
nix develop
cargo build --locked --profile release-with-debug
picotool uf2 convert target/thumbv8m.main-none-eabihf/release-with-debug/cec-4k cec-4k.uf2
```

If direnv is enabled in your shell, run `direnv allow` once in this directory to enter the flake dev shell automatically.

For a reproducible production artifact:

```sh
nix build .#firmware
ls result/cec-4k.{uf2,elf}
```

Format the Nix and Rust sources with `nix fmt`.

## Flash

Hold the XIAO RP2350's BOOT button while connecting its USB cable, then release the button to enter BOOTSEL mode. From the dev shell, flash the UF2 built above and reboot into the firmware:

```sh
picotool load -v -x cec-4k.uf2
```

If you used `nix build .#firmware`, flash `result/cec-4k.uf2` instead.

After flashing a firmware with the diagnostic command, `python3 tools/ddc_diag.py --bootsel` enters BOOTSEL over the USB serial connection for the next update. The normal USB connection does not provide JTAG or SWD.

## fwupd on NixOS

The firmware exposes Raspberry Pi's USB reset interface alongside CDC ACM. fwupd's existing `rp-pico` plugin uses that interface to enter RP2350 BOOTSEL, then its `uf2` plugin writes the UF2 image. The overlay adds device matching for this adapter and the RP2350 ROM's `2e8a:000f` USB identity.

In a NixOS flake, include this repository as an input and add `inputs.cec-4k.nixosModules.fwupd` to the host's module list. Alternatively, add `inputs.cec-4k.overlays.default` to `nixpkgs.overlays` and enable `services.fwupd.enable = true;`. Rebuild the NixOS configuration, then flash a firmware containing the USB reset interface once using the manual method above. `fwupdmgr get-devices` should then show the adapter as updatable. A signed or local fwupd CAB containing the UF2 and release metadata is still required for `fwupdmgr update` to offer an update.

The runtime quirk selects fwupd's `rp-pico` plugin by VID:PID; that plugin also checks for the USB reset interface, which ordinary Pulse-Eight adapters lack. Increment the firmware's USB `device_release` for future firmware versions so fwupd can report the installed version. The RP2350 ROM BOOTSEL button remains a recovery path.

## DDC diagnostics

Run `python3 tools/ddc_diag.py` for a snapshot or add `--watch` while reading the monitor's EDID. It reports live SDA/SCL levels, PIO START/STOP and byte counts, FIFO stalls, ring overflows, EDID decoder counts, and the first two raw words after the latest START. This command uses firmware-specific serial code `0x40` and does not require the Linux CEC driver.

The onboard RGB LED uses GPIO22 for data and GPIO23 for power. Blue means the firmware is running but has not captured a DDC START. Amber means DDC traffic was captured but no physical address was found. Red means a FIFO stall or SRAM ring overflow occurred. Green means the physical address was found. The serial diagnostics give the exact counters and GPIO levels.

For an active EDID read from the HDMI connector, use `nix run nixpkgs#ddcutil -- --edid-read-size=256 --disable-try-get-edid-from-sysfs detect` while the adapter is in the HDMI path. Compare snapshots before and after the read. A guarded serial command (`0x41` with literal payload `RP25`) enters BOOTSEL; `tools/ddc_diag.py --bootsel` sends it.

## Connections

| XIAO pin | RP2350 GPIO | Role |
| --- | ---: | --- |
| D9 | GPIO4 | Bidirectional CEC, open drain |
| D4 | GPIO6 | DDC SDA input only |
| D5 | GPIO7 | DDC SCL input only |

D4 and D5 are receive-only GPIO inputs, not I²C master pins. Use a 100kΩ resistor between the HDMI bus and the RP2350.

## Firmware structure

- [`src/main.rs`](src/main.rs): RP2350 clocks, pins, USB CDC, and core startup.
- [`src/cec.rs`](src/cec.rs): CEC timing engine on core 1.
- [`src/ddc.rs`](src/ddc.rs): passive PIO capture and an interrupt that copies I²C records into a 512-word SRAM ring.
- [`src/ddc_protocol.rs`](src/ddc_protocol.rs): foreground I²C/EDID decoding and CTA HDMI VSDB physical address extraction.
- [`src/pulse8.rs`](src/pulse8.rs): Pulse-Eight serial framing and Linux driver command subset.
- [`src/transport.rs`](src/transport.rs): short critical sections for cross-core messages.

The USB device uses Pulse-Eight VID:PID `2548:1002`, as specified in `details.txt`, and reports `CEC 4K` / `RP2350 HDMI CEC Adapter` as its USB manufacturer and product strings. Its USB serial number is the RP2350's 64-bit chip ID in hexadecimal; if the ID cannot be read, the serial descriptor is omitted. On Linux, attach the in-tree driver with `inputattach --pulse8-cec /dev/ttyACM0`; the TTY name may differ. Setting the serial line discipline may require root privileges. The kernel CEC device should then appear as `/dev/cec*`. If an autoattach udev rule matches the manufacturer or product strings, update it to match these strings or use the VID:PID instead.

The DDC sniffer only learns an address when it sees the host read the relevant EDID extension. A PIO FIFO stall or SRAM ring overflow discards the incomplete capture and waits for a new I²C START. DDC timing and capture still need hardware testing.
