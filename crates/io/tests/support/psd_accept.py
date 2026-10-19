#!/usr/bin/env python3
"""Third-party PSD reader for PhotoCraft's export acceptance test (FILE-216-4).

Reads PSD/PSB files with psd-tools (MIT, https://github.com/psd-tools/psd-tools) and prints
one JSON object per file: what an independent reader makes of the file. The Rust test
(`crates/io/tests/psd_acceptance.rs`) compares that with what PhotoCraft's own reader sees,
so a file that only PhotoCraft can read is a failure.

Usage:
    python3 psd_accept.py FILE...        # one JSON object per line
    python3 psd_accept.py --composite FILE...   # also composite the file (slower)

Output per file:
    {"file": str, "ok": bool, "error": str|null, "width": int, "height": int, "depth": int,
     "color_mode": str, "composite": bool, "layers": [
         {"name": str, "kind": str, "visible": bool, "group": bool}]}

The layer list is the flattened tree, in the order psd-tools walks it (bottom-most first).
"""
import json
import sys


def describe(path, composite):
    out = {"file": path, "ok": False, "error": None, "layers": []}
    try:
        from psd_tools import PSDImage

        psd = PSDImage.open(path)
        out["ok"] = True
        out["width"] = psd.width
        out["height"] = psd.height
        out["depth"] = psd.depth
        out["color_mode"] = str(psd.color_mode)
        for layer in psd.descendants():
            out["layers"].append(
                {
                    "name": layer.name,
                    "kind": layer.kind,
                    "visible": bool(layer.visible),
                    "group": bool(layer.is_group()),
                }
            )
        if composite:
            image = psd.composite()
            out["composite"] = image is not None
    except Exception as exc:  # noqa: BLE001 - reporting the failure is the whole point
        out["error"] = f"{type(exc).__name__}: {exc}"
    return out


def main(argv):
    composite = False
    args = argv[1:]
    if args and args[0] == "--composite":
        composite = True
        args = args[1:]
    for path in args:
        print(json.dumps(describe(path, composite)))


if __name__ == "__main__":
    main(sys.argv)
