#!/usr/bin/env python3
"""Draws the VizCraft app icon (the gradient tile with four white bars, as in
apps/vizcraft/src/icons.rs::app_icon) as a PNG at any size, with anti-aliased edges.
No dependencies beyond the Python standard library.

    python3 make_icon.py 1024 icon_1024.png
"""
import math
import struct
import sys
import zlib


def rounded_rect_sdf(px, py, cx, cy, hw, hh, r):
    """Signed distance from a point to a rounded rectangle (negative inside)."""
    qx = abs(px - cx) - (hw - r)
    qy = abs(py - cy) - (hh - r)
    ox, oy = max(qx, 0.0), max(qy, 0.0)
    return math.hypot(ox, oy) + min(max(qx, qy), 0.0) - r


def coverage(d):
    return min(max(0.5 - d, 0.0), 1.0)


def draw(n):
    s = n / 64.0
    # macOS icon grid: the tile fills ~80 % of the canvas, with room for the shadow.
    margin = n * 0.1
    tile = n - 2 * margin
    ts = tile / 64.0
    c = n / 2.0
    radius = 14.0 * ts
    top = (0x1E, 0x3A, 0x8A)
    bottom = (0x14, 0xB8, 0xA6)
    # Bars in the 64-grid of the tile: (center x, top y), bottom at 50, width 8, radius 2.
    bars = [(14, 38), (26, 26), (38, 32), (50, 16)]
    rows = []
    for y in range(n):
        row = bytearray()
        py = y + 0.5
        for x in range(n):
            px = x + 0.5
            d = rounded_rect_sdf(px, py, c, c, tile / 2, tile / 2, radius)
            a_tile = coverage(d)
            # Soft drop shadow under the tile.
            ds = rounded_rect_sdf(px, py - n * 0.012, c, c, tile / 2, tile / 2, radius)
            shadow = max(0.0, 1.0 - max(ds, 0.0) / (n * 0.03)) * 0.28 if ds > -1 else 0.0
            if a_tile <= 0.0 and shadow <= 0.0:
                row += b"\x00\x00\x00\x00"
                continue
            # Diagonal gradient.
            t = min(max(((px - margin) + (py - margin)) / (2 * tile), 0.0), 1.0)
            col = [top[i] + (bottom[i] - top[i]) * t for i in range(3)]
            # Subtle top highlight.
            hl = max(0.0, 1.0 - (py - margin) / (tile * 0.5)) * 0.10
            col = [v + (255 - v) * hl for v in col]
            # Bars.
            gx = (px - margin) / ts
            gy = (py - margin) / ts
            bar = 0.0
            for bx, btop in bars:
                bd = rounded_rect_sdf(gx, gy, bx, (btop + 50) / 2.0, 4.0, (50 - btop) / 2.0, 1.6) * ts
                bar = max(bar, coverage(bd))
            col = [v + (255 - v) * bar for v in col]
            a = a_tile + shadow * (1 - a_tile)
            if a <= 0:
                row += b"\x00\x00\x00\x00"
                continue
            # Composite over the shadow (black).
            out = [v * a_tile / a for v in col]
            row += bytes([int(round(min(max(v, 0), 255))) for v in out]) + bytes([int(round(a * 255))])
        rows.append(bytes(row))
    _ = s
    return rows


def write_png(path, n, rows):
    raw = b"".join(b"\x00" + r for r in rows)

    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", n, n, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)


if __name__ == "__main__":
    size = int(sys.argv[1]) if len(sys.argv) > 1 else 1024
    out = sys.argv[2] if len(sys.argv) > 2 else "icon_%d.png" % size
    write_png(out, size, draw(size))
    print("Wrote", out)
