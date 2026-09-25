# CEC 4K RP2350 firmware

Rust firmware for a Seeed XIAO RP2350 inline HDMI CEC adapter. The build produces a UF2 and ELF using pinned Nix and Cargo dependencies.

## Build

```sh
nix develop
cargo build --locked --profile release-with-debug
picotool uf2 convert target/thumbv8m.main-none-eabihf/release-with-debug/cec-4k cec-4k.uf2
```

For a reproducible production artifact:

```sh
nix build .#firmware
ls result/cec-4k.{uf2,elf}
```

Format the Nix and Rust sources with `nix fmt`.

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

The USB device uses Pulse-Eight VID:PID `2548:1002`, as specified in `details.txt`, and reports `CEC 4K` / `RP2350 HDMI CEC Adapter` as its USB manufacturer and product strings. Its USB serial number is the RP2350's 64-bit chip ID in hexadecimal; if the ID cannot be read, the serial descriptor is omitted. On Linux, attach the in-tree driver with `inputattach --pulse8-cec /dev/ttyACM0`; the TTY name may differ. The kernel CEC device should then appear as `/dev/cec*`. If an autoattach udev rule matches the manufacturer or product strings, update it to match these strings or use the VID:PID instead.

The DDC sniffer only learns an address when it sees the host read the relevant EDID extension. A PIO FIFO stall or SRAM ring overflow discards the incomplete capture and waits for a new I²C START. DDC timing and capture still need hardware testing.
