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

The Rust toolchain is pinned to 1.98.1. `release-with-debug` keeps symbols in the ELF while optimizing the firmware.

## Connections

| XIAO pin | RP2350 GPIO | Role |
| --- | ---: | --- |
| D9 | GPIO4 | Bidirectional CEC, open drain |
| D4 | GPIO6 | DDC SDA input only |
| D5 | GPIO7 | DDC SCL input only |

D9 must connect to HDMI CEC pin 13 through a **bidirectional, open-drain-safe CEC interface**. The firmware releases the line for logic high and only pulls it low. It must be able to read the actual bus level during arbitration and ACK. Connect HDMI DDC/CEC ground (pin 17) to the board ground. Keep the HDMI data and DDC lines passing through the inline adapter.

D4 and D5 are receive-only GPIO inputs, not I²C master pins. Level shift or divide the HDMI DDC voltage before these pins. A prototype divider is 100 kΩ from each DDC line to its GPIO and 150 kΩ from that GPIO to ground. Do not wire a 5 V DDC line directly to an RP2350 GPIO. The sniffer never drives either DDC line.

## Firmware structure

- [`src/main.rs`](src/main.rs): RP2350 clocks, pins, USB CDC, and core startup.
- [`src/cec.rs`](src/cec.rs): CEC timing engine on core 1.
- [`src/ddc.rs`](src/ddc.rs): passive DDC edge decoding and CTA HDMI VSDB physical address extraction.
- [`src/pulse8.rs`](src/pulse8.rs): Pulse-Eight serial framing and Linux driver command subset.
- [`src/transport.rs`](src/transport.rs): short critical sections for cross-core messages.

The USB device uses Pulse-Eight VID:PID `2548:1002`, as specified in `details.txt`. On Linux, attach the in-tree driver with `inputattach --pulse8-cec /dev/ttyACM0`; the TTY name may differ. The kernel CEC device should then appear as `/dev/cec*`.

## Current limits

This is a compiling firmware implementation, **not a hardware-validated CEC adapter**. CEC electrical behavior, receive/ACK timing, arbitration, DDC capture at the attached HDMI link's speed, and Linux interoperability still need bench testing. The DDC sniffer only learns an address when it sees the host read the relevant EDID extension; it does not initiate an EDID read.

The upstream Linux `pulse8-cec` driver does not automatically use a newly sniffed EDID address from firmware version 1. For a first Linux setup, set the adapter's physical address from the corresponding DRM connector's EDID using `cec-ctl -E /sys/class/drm/<connector>/edid`, as documented by the kernel. The firmware exposes the sniffed address to the Pulse-Eight `GET_PHYSICAL_ADDRESS` command for future integration, but this alone does not update `/dev/cec*` on the default driver path.
