#!/usr/bin/env python3
# Writes the placeholder web app icons icon-192.png, icon-512.png and
# icon-512-maskable.png into the given directory, byte-identically per run.
import struct
import sys
import zlib
from pathlib import Path

ICONS = (
    ("icon-192.png", 192, 42),
    ("icon-512.png", 512, 42),
    ("icon-512-maskable.png", 512, 30),
)
BACKGROUND = (0x10, 0x15, 0x16)
MARK = (0x69, 0xD1, 0xBA)


def chunk(kind, payload):
    length = struct.pack(">I", len(payload))
    crc = struct.pack(">I", zlib.crc32(kind + payload) & 0xFFFFFFFF)
    return length + kind + payload + crc


def png(size, percent):
    mark = bytes(MARK)
    background = bytes(BACKGROUND)
    raw = bytearray()
    for y in range(size):
        row = bytearray((0,))
        for x in range(size):
            disc = 10000 * ((2 * x + 1 - size) ** 2 + (2 * y + 1 - size) ** 2)
            row += mark if disc <= (2 * percent * size) ** 2 else background
        raw += row
    ihdr = struct.pack(">IIBBBBB", size, size, 8, 2, 0, 0, 0)
    idat = zlib.compress(bytes(raw), 9)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr)
            + chunk(b"IDAT", idat) + chunk(b"IEND", b""))


def main(argv):
    if len(argv) != 2:
        print("usage: python3 -I make_app_icons.py <output directory>", file=sys.stderr)
        return 2
    out_dir = Path(argv[1])
    out_dir.mkdir(parents=True, exist_ok=True)
    for name, size, percent in ICONS:
        data = png(size, percent)
        out_dir.joinpath(name).write_bytes(data)
        print(f"{name} {size}x{size} {len(data)} bytes")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
