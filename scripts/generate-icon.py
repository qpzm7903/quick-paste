#!/usr/bin/env python3
"""使用 Python 标准库与 macOS 自带 sips/iconutil 生成 QuickPaste.icns 应用图标。"""

import argparse
import math
import os
import shutil
import struct
import subprocess
import tempfile
import zlib


def sdf_rounded_rect(px: float, py: float, x0: float, y0: float, x1: float, y1: float, r: float) -> float:
    cx = (x0 + x1) * 0.5
    cy = (y0 + y1) * 0.5
    hx = (x1 - x0) * 0.5 - r
    hy = (y1 - y0) * 0.5 - r
    dx = abs(px - cx) - hx
    dy = abs(py - cy) - hy
    outside = math.hypot(max(dx, 0.0), max(dy, 0.0))
    inside = min(max(dx, dy), 0.0)
    return outside + inside - r


def clamp01(v: float) -> float:
    return 0.0 if v <= 0.0 else (1.0 if v >= 1.0 else v)


def blend(bg: tuple[float, float, float, float], fg: tuple[float, float, float, float], alpha: float) -> tuple[float, float, float, float]:
    a = clamp01(alpha) * fg[3]
    out_a = a + bg[3] * (1.0 - a)
    if out_a <= 1e-6:
        return (0.0, 0.0, 0.0, 0.0)
    r = (fg[0] * a + bg[0] * bg[3] * (1.0 - a)) / out_a
    g = (fg[1] * a + bg[1] * bg[3] * (1.0 - a)) / out_a
    b = (fg[2] * a + bg[2] * bg[3] * (1.0 - a)) / out_a
    return (r, g, b, out_a)


def render_master_png(path: str, size: int = 512) -> None:
    aa = 1.6 / size
    raw_rows = bytearray()

    for y in range(size):
        raw_rows.append(0)  # PNG filter type 0
        ny = (y + 0.5) / size
        for x in range(size):
            nx = (x + 0.5) / size

            # macOS 图标内边距与圆角底板
            bg_dist = sdf_rounded_rect(nx, ny, 0.08, 0.08, 0.92, 0.92, 0.19)
            bg_alpha = clamp01((0.0 - bg_dist) / aa + 0.5)
            if bg_alpha <= 0.0:
                raw_rows.extend((0, 0, 0, 0))
                continue

            # 蓝靛渐变底色 (#2563EB -> #4F46E5)
            t = clamp01((nx * 0.45 + ny * 0.55))
            base = (
                37.0 + (79.0 - 37.0) * t,
                99.0 + (70.0 - 99.0) * t,
                235.0 + (229.0 - 235.0) * t,
                bg_alpha,
            )

            # 白色剪贴板纸张主体
            board_dist = sdf_rounded_rect(nx, ny, 0.27, 0.24, 0.73, 0.79, 0.065)
            board_alpha = clamp01((0.0 - board_dist) / aa + 0.5)
            color = blend(base, (248.0, 250.0, 252.0, 0.96), board_alpha)

            # 顶部金属夹扣
            clip_dist = sdf_rounded_rect(nx, ny, 0.375, 0.185, 0.625, 0.295, 0.038)
            clip_alpha = clamp01((0.0 - clip_dist) / aa + 0.5)
            color = blend(color, (191.0, 219.0, 254.0, 1.0), clip_alpha)

            # 内部三条文本横线
            for ly, rx in ((0.41, 0.63), (0.525, 0.63), (0.64, 0.53)):
                line_dist = sdf_rounded_rect(nx, ny, 0.36, ly, rx, ly + 0.052, 0.025)
                line_alpha = clamp01((0.0 - line_dist) / aa + 0.5)
                if line_alpha > 0.0:
                    color = blend(color, (59.0, 130.0, 246.0, 0.88), line_alpha)

            raw_rows.extend((
                int(max(0, min(255, round(color[0])))),
                int(max(0, min(255, round(color[1])))),
                int(max(0, min(255, round(color[2])))),
                int(max(0, min(255, round(color[3] * 255.0)))),
            ))

    write_png(path, size, size, bytes(raw_rows))


def png_chunk(chunk_type: bytes, data: bytes) -> bytes:
    payload = chunk_type + data
    return struct.pack(">I", len(data)) + payload + struct.pack(">I", zlib.crc32(payload) & 0xFFFFFFFF)


def write_png(path: str, width: int, height: int, raw_scanlines: bytes) -> None:
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    idat = zlib.compress(raw_scanlines, level=6)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(png_chunk(b"IHDR", ihdr))
        f.write(png_chunk(b"IDAT", idat))
        f.write(png_chunk(b"IEND", b""))


def build_icns(output_icns: str) -> None:
    with tempfile.TemporaryDirectory() as tmp:
        master_png = os.path.join(tmp, "master.png")
        iconset_dir = os.path.join(tmp, "QuickPaste.iconset")
        os.makedirs(iconset_dir, exist_ok=True)
        render_master_png(master_png, 512)

        targets = [
            ("icon_16x16.png", 16),
            ("icon_16x16@2x.png", 32),
            ("icon_32x32.png", 32),
            ("icon_32x32@2x.png", 64),
            ("icon_128x128.png", 128),
            ("icon_128x128@2x.png", 256),
            ("icon_256x256.png", 256),
            ("icon_256x256@2x.png", 512),
            ("icon_512x512.png", 512),
            ("icon_512x512@2x.png", 1024),
        ]
        for filename, px in targets:
            out_png = os.path.join(iconset_dir, filename)
            if px == 512:
                shutil.copyfile(master_png, out_png)
            else:
                subprocess.run(
                    ["sips", "-z", str(px), str(px), master_png, "--out", out_png],
                    check=True,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )

        os.makedirs(os.path.dirname(os.path.abspath(output_icns)), exist_ok=True)
        subprocess.run(["iconutil", "-c", "icns", iconset_dir, "-o", output_icns], check=True)


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate QuickPaste macOS .icns icon")
    parser.add_argument("output", help="Output .icns file path")
    args = parser.parse_args()
    build_icns(args.output)


if __name__ == "__main__":
    main()
