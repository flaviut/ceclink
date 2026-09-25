#!/usr/bin/env python3
"""Generate a JLCPCB-ready gerber + drill zip from a KiCad project."""
from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path

FAB_LAYERS = [
    "F.Paste", "B.Paste",
    "F.Silkscreen", "B.Silkscreen",
    "F.Mask", "B.Mask",
    "Edge.Cuts",
]


def find_pcb(project_dir: Path) -> Path:
    pcbs = list(project_dir.glob("*.kicad_pcb"))
    if len(pcbs) != 1:
        sys.exit(f"expected exactly one .kicad_pcb in {project_dir}, found {len(pcbs)}")
    return pcbs[0]


def detect_copper_layers(pcb: Path) -> list[str]:
    """Parse the (layers ...) block and return signal-layer names in order."""
    text = pcb.read_text()
    match = re.search(r"\(layers\b(.*?)^\s*\)", text, re.DOTALL | re.MULTILINE)
    if not match:
        sys.exit(f"could not find (layers ...) block in {pcb}")
    layers = re.findall(r'"([^"]+)"\s+signal\b', match.group(1))
    if not layers:
        sys.exit(f"no signal (copper) layers found in {pcb}")
    return layers


def run_kicad(args: list[str]) -> None:
    print("==>", " ".join(args))
    subprocess.run(["kicad-cli", *args], check=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("project_dir", type=Path)
    opts = parser.parse_args()

    project_dir = opts.project_dir.resolve()
    pcb = find_pcb(project_dir)
    name = pcb.stem
    out_dir = project_dir / "fab"
    out_dir.mkdir(exist_ok=True)
    zip_path = out_dir / f"{name}_jlcpcb.zip"

    copper = detect_copper_layers(pcb)
    layers = ",".join(copper + FAB_LAYERS)
    print(f"==> layers: {layers}")

    with tempfile.TemporaryDirectory(prefix=f"jlcpcb_{name}_") as tmp:
        work_dir = Path(tmp)
        run_kicad([
            "pcb", "export", "gerbers",
            "--output", f"{work_dir}/",
            "--layers", layers,
            "--no-x2",
            "--subtract-soldermask",
            str(pcb),
        ])
        run_kicad([
            "pcb", "export", "drill",
            "--output", f"{work_dir}/",
            "--format", "excellon",
            "--drill-origin", "absolute",
            "--excellon-units", "mm",
            "--excellon-zeros-format", "decimal",
            "--excellon-oval-format", "alternate",
            str(pcb),
        ])

        zip_path.unlink(missing_ok=True)
        with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED) as z:
            for f in sorted(work_dir.iterdir()):
                if f.is_file():
                    z.write(f, f.name)
    print(f"done: {zip_path}")


if __name__ == "__main__":
    main()
