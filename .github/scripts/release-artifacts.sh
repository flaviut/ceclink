#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."

output_dir="${1:-dist}"
mkdir -p "$output_dir"

firmware_out="$(nix build .#firmware --no-link --print-out-paths)"
test -s "$firmware_out/ceclink.uf2"
cp "$firmware_out/ceclink.uf2" "$output_dir/firmware.uf2"

python3 jlcpcb_fab.py pcb/xiao-rp2350-adaptor
python3 jlcpcb_fab.py pcb/hdmi-breakout

test -s pcb/xiao-rp2350-adaptor/fab/xiao-rp2350-adaptor_jlcpcb.zip
test -s pcb/hdmi-breakout/fab/hdmi-breakout_jlcpcb.zip
cp pcb/xiao-rp2350-adaptor/fab/xiao-rp2350-adaptor_jlcpcb.zip "$output_dir/adapter.zip"
cp pcb/hdmi-breakout/fab/hdmi-breakout_jlcpcb.zip "$output_dir/hdmi-breakout.zip"
