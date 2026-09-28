#!/usr/bin/env python3
"""Generates the bundled Espectral cosmetic textures (MOD-RENDER slice).

Writes commit-ready RGBA PNGs into
``branding-mod/src/main/resources/assets/espectral-menu/textures/cosmetics/``.
Pure standard library (zlib + struct): deterministic output, no Pillow, no
network. Art direction: dark spectral charcoal with gold accents; every pixel
keeps alpha >= 200 so no cosmetic can render below the minimum opacity of the
footprint cap (wings are the most translucent at ~200).

Usage:  python scripts/generate_cosmetic_textures.py [outdir]
"""

import math
import os
import struct
import sys
import zlib

# ----------------------------------------------------------------------------
# PNG writer
# ----------------------------------------------------------------------------


def write_png(path, w, h, rows):
    """rows: list of h rows, each a list of w (r, g, b, a) tuples."""

    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    raw = bytearray()
    for row in rows:
        raw.append(0)  # filter: none
        for r, g, b, a in row:
            raw += bytes((r & 0xFF, g & 0xFF, b & 0xFF, a & 0xFF))
    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
    png += chunk(b"IEND", b"")
    with open(path, "wb") as fh:
        fh.write(png)


# ----------------------------------------------------------------------------
# Canvas + tiny art helpers
# ----------------------------------------------------------------------------


class Img:
    def __init__(self, w, h, rgba=(20, 22, 30, 255)):
        self.w, self.h = w, h
        self.px = [[tuple(rgba) for _ in range(w)] for _ in range(h)]

    def set(self, x, y, rgba):
        if 0 <= x < self.w and 0 <= y < self.h:
            self.px[y][x] = (rgba[0] & 0xFF, rgba[1] & 0xFF, rgba[2] & 0xFF, rgba[3] & 0xFF)

    def get(self, x, y):
        return self.px[y % self.h][x % self.w]

    def blend(self, x, y, rgba, k=1.0):
        r0, g0, b0, a0 = self.get(x, y)
        r = int(r0 * (1 - k) + rgba[0] * k)
        g = int(g0 * (1 - k) + rgba[1] * k)
        b = int(b0 * (1 - k) + rgba[2] * k)
        a = int(a0 * (1 - k) + rgba[3] * k)
        self.set(x, y, (r, g, b, max(200, a)))

    def rect(self, x0, y0, x1, y1, rgba, k=1.0):
        for y in range(int(y0), int(y1)):
            for x in range(int(x0), int(x1)):
                self.blend(x, y, rgba, k)

    def vgrad(self, top, bottom, y0=0, y1=None, k=1.0):
        y1 = self.h if y1 is None else y1
        span = max(1, y1 - y0 - 1)
        for y in range(int(y0), int(y1)):
            t = (y - y0) / span
            col = tuple(int(top[i] * (1 - t) + bottom[i] * t) for i in range(4))
            for x in range(self.w):
                self.blend(x, y, col, k)

    def glow(self, cx, cy, rad, rgba, strength=1.0):
        for y in range(int(cy - rad), int(cy + rad) + 1):
            for x in range(int(cx - rad), int(cx + rad) + 1):
                d = math.hypot(x - cx, y - cy)
                if d <= rad:
                    self.blend(x, y, rgba, (1.0 - d / rad) ** 2 * strength)


def hash2(ix, iy, seed=0):
    n = (ix * 374761393 + iy * 668265263 + seed * 1442695041) & 0xFFFFFFFF
    n = (n ^ (n >> 13)) * 1274126177 & 0xFFFFFFFF
    return ((n ^ (n >> 16)) & 0xFFFF) / 65535.0


def vnoise(x, y, seed=0):
    ix, iy = math.floor(x), math.floor(y)
    fx, fy = x - ix, y - iy
    fx = fx * fx * (3 - 2 * fx)
    fy = fy * fy * (3 - 2 * fy)
    a = hash2(ix, iy, seed)
    b = hash2(ix + 1, iy, seed)
    c = hash2(ix, iy + 1, seed)
    d = hash2(ix + 1, iy + 1, seed)
    return (a * (1 - fx) + b * fx) * (1 - fy) + (c * (1 - fx) + d * fx) * fy


def fbm(x, y, octaves=4, seed=0):
    total, amp, freq, norm = 0.0, 1.0, 1.0, 0.0
    for i in range(octaves):
        total += vnoise(x * freq, y * freq, seed + i * 17) * amp
        norm += amp
        amp *= 0.5
        freq *= 2.0
    return total / norm


def grain(img, seed=0, strength=10, scale=0.55):
    for y in range(img.h):
        for x in range(img.w):
            n = fbm(x * scale, y * scale, 3, seed) - 0.5
            k = int(n * 2 * strength)
            r, g, b, a = img.get(x, y)
            img.set(x, y, (r + k, g + k, b + k, a))


def edge_trim(img, rgba, thick=2, inset=0):
    for y in range(img.h):
        for x in range(img.w):
            d = min(x - inset, y - inset, img.w - 1 - x - inset, img.h - 1 - y - inset)
            if d < thick:
                img.blend(x, y, rgba, 1.0 - d / thick * 0.65)


def vertical_bands(img, x0, x1, rgba, spacing, thick=1, k=0.5):
    x = x0
    while x < x1:
        img.rect(x, 0, x + thick, img.h, rgba, k)
        x += spacing


def runes(img, x0, y0, x1, y1, rgba, seed=7, k=0.9):
    """Small geometric glyph strokes (runic wraps)."""
    rnd = seed
    y = y0
    while y < y1:
        x = x0
        while x < x1:
            rnd = (rnd * 1103515245 + 12345) & 0x7FFFFFFF
            kind = (rnd >> 7) % 4
            if kind == 0:
                for i in range(4):
                    img.blend(x + i, y, rgba, k)
                    img.blend(x + i, y + 3, rgba, k * 0.8)
            elif kind == 1:
                for i in range(4):
                    img.blend(x, y + i, rgba, k)
                img.blend(x + 2, y + 1, rgba, k)
                img.blend(x + 3, y + 2, rgba, k)
            elif kind == 2:
                for i in range(3):
                    img.blend(x + i, y + i, rgba, k)
                    img.blend(x + i, y + 3 - i, rgba, k * 0.8)
            else:
                img.blend(x, y, rgba, k)
                img.blend(x + 1, y, rgba, k)
                img.blend(x + 1, y + 1, rgba, k)
                img.blend(x + 2, y + 2, rgba, k * 0.8)
            x += 7
        y += 9


# ----------------------------------------------------------------------------
# Palette
# ----------------------------------------------------------------------------

DARK = (18, 20, 28, 255)
DARK2 = (32, 34, 46, 255)
SHADE = (26, 28, 38, 255)
GOLD = (232, 182, 76, 255)
GOLD_HI = (255, 214, 130, 255)
GOLD_DIM = (150, 112, 48, 255)
SPECTRAL = (52, 46, 82, 255)
EMBER = (214, 120, 52, 255)


# ----------------------------------------------------------------------------
# Recipes
# ----------------------------------------------------------------------------


def mantle(out):
    img = Img(64, 64)
    img.vgrad(DARK2, DARK)
    # Diagonal gold sash across the sheet (the one-shoulder drape).
    for y in range(img.h):
        for x in range(img.w):
            band = abs((x - y * 0.55) - 26)
            if band < 7:
                img.blend(x, y, GOLD_DIM, (7 - band) / 7 * 0.85)
            if band < 2.2:
                img.blend(x, y, GOLD, 0.9)
    grain(img, seed=3, strength=12)
    # Gold clasp motif near the top-right (collarbone).
    img.glow(52, 10, 7, GOLD_HI, 0.8)
    img.rect(49, 7, 55, 13, GOLD, 0.85)
    img.rect(50, 8, 54, 12, GOLD_HI, 0.9)
    # Hem band with gold mist.
    img.rect(0, 54, 64, 58, GOLD_DIM, 0.55)
    img.rect(0, 58, 64, 64, GOLD, 0.35)
    for x in range(64):
        for y in range(54, 64):
            img.blend(x, y, GOLD_HI, (y - 54) / 10 * 0.25)
    edge_trim(img, GOLD_DIM, thick=2)
    write_png(os.path.join(out, "manto_espectral.png"), img.w, img.h, img.px)


def revenant(out):
    img = Img(64, 64)
    img.vgrad(SHADE, DARK)
    vertical_bands(img, 0, 64, SPECTRAL, 16, 1, 0.35)
    grain(img, seed=11, strength=14)
    # Gold piping along both hems (u = 0 and u = 1 are the sheet sides).
    for y in range(img.h):
        for x in range(img.w):
            d = min(x, img.w - 1 - x)
            if d < 3:
                img.blend(x, y, GOLD, (3 - d) / 3 * 0.85)
    # Tattered hem: dark bites + gold mist.
    for x in range(img.w):
        bite = int(4 + 5 * vnoise(x * 0.35, 0, 21))
        img.rect(x, 64 - bite, x + 1, 64, (18, 20, 28, 255), 0.0)
        for y in range(64 - bite, 64):
            img.blend(x, y, GOLD_DIM, (y - (64 - bite)) / max(1, bite) * 0.5)
    edge_trim(img, GOLD_DIM, thick=1)
    write_png(os.path.join(out, "capucha_errante.png"), img.w, img.h, img.px)


def revenant_hood(out):
    img = Img(64, 32)
    img.vgrad(SHADE, DARK)
    grain(img, seed=13, strength=12)
    for y in range(img.h):
        for x in range(img.w):
            d = min(y, img.h - 1 - y)
            if d < 2:
                img.blend(x, y, GOLD, (2 - d) / 2 * 0.8)
    img.rect(0, 14, 64, 17, GOLD_DIM, 0.4)
    edge_trim(img, GOLD_DIM, thick=1)
    write_png(os.path.join(out, "capucha_errante_hood.png"), img.w, img.h, img.px)


def tatters(out):
    img = Img(32, 64)
    img.vgrad(SHADE, DARK)
    grain(img, seed=17, strength=16, scale=0.8)
    # Smoke wisps.
    for y in range(img.h):
        for x in range(img.w):
            n = fbm(x * 0.12, y * 0.07, 3, 31)
            if n > 0.58:
                img.blend(x, y, SPECTRAL, (n - 0.58) * 1.6)
    # Gold inner edge along the ribbon's left side.
    img.rect(0, 0, 5, 64, GOLD_DIM, 0.55)
    img.rect(1, 0, 3, 64, GOLD, 0.8)
    img.rect(27, 0, 32, 64, GOLD_DIM, 0.35)
    write_png(os.path.join(out, "jirones_ceniza.png"), img.w, img.h, img.px)


def wings(out):
    img = Img(64, 32, (26, 28, 40, 210))
    img.vgrad((58, 52, 96, 225), (26, 28, 42, 205))
    # Soft spectral glow near the wing root.
    img.glow(8, 16, 16, (96, 84, 150, 255), 0.55)
    # Feather quills: bold, slightly curved, gold-dim with a bright core.
    for i in range(5):
        y0 = 3 + i * 6
        for x in range(img.w):
            y = y0 + int(x * 0.16)
            for k in range(3):
                if 0 <= y + k < img.h:
                    img.blend(x, y + k, GOLD_DIM, 0.55 - k * 0.18)
            if 0 <= y + 1 < img.h:
                img.blend(x, y + 1, GOLD, 0.4)
    # One gold vein along the wing spine.
    for x in range(img.w):
        y = 15 + int(2 * math.sin(x * 0.12))
        img.blend(x, y, GOLD_HI, 0.55)
        img.blend(x, y + 1, GOLD, 0.35)
    grain(img, seed=23, strength=4)
    edge_trim(img, GOLD, thick=1)
    write_png(os.path.join(out, "alas_eco.png"), img.w, img.h, img.px)


def scarf(out):
    img = Img(64, 32)
    img.vgrad(DARK2, DARK)
    grain(img, seed=29, strength=12)
    for y in range(img.h):
        for x in range(img.w):
            d = min(y, img.h - 1 - y)
            if d < 2:
                img.blend(x, y, GOLD, (2 - d) / 2 * 0.75)
    vertical_bands(img, 0, 64, GOLD_DIM, 11, 1, 0.35)
    write_png(os.path.join(out, "bufanda_llama_alma.png"), img.w, img.h, img.px)


def scarf_tail(out):
    img = Img(32, 32)
    img.vgrad(DARK2, SHADE)
    grain(img, seed=31, strength=12)
    img.rect(0, 0, 3, 32, GOLD_DIM, 0.5)
    img.rect(29, 0, 32, 32, GOLD_DIM, 0.5)
    # Flame edge at the hem (gold, brighter toward the tip).
    for y in range(18, 32):
        k = (y - 18) / 14
        for x in range(32):
            wob = 2.0 * math.sin(x * 0.5 + y * 0.4)
            if y > 18 + wob:
                img.blend(x, y, GOLD, 0.35 + k * 0.5)
                if y > 26 + wob:
                    img.blend(x, y, GOLD_HI, 0.6)
    write_png(os.path.join(out, "bufanda_llama_alma_tail.png"), img.w, img.h, img.px)


def ears(out):
    img = Img(64, 32)
    img.vgrad((44, 36, 32, 255), (28, 24, 26, 255))
    grain(img, seed=37, strength=14, scale=0.9)
    # Fur streaks.
    for i in range(14):
        x = (i * 17) % 64
        for y in range(img.h):
            img.blend((x + int(y * 0.35)) % 64, y, GOLD_DIM, 0.12)
    # Gold inner-ear glow.
    img.glow(16, 12, 9, GOLD, 0.55)
    img.glow(48, 12, 9, GOLD, 0.55)
    edge_trim(img, GOLD_DIM, thick=1)
    write_png(os.path.join(out, "orejas_vulpinas.png"), img.w, img.h, img.px)


def wraps(out):
    img = Img(64, 32)
    img.vgrad((38, 34, 30, 255), (24, 22, 24, 255))
    grain(img, seed=41, strength=12)
    # Cloth wrap ridges.
    for y in range(img.h):
        for x in range(img.w):
            if ((x + y * 2) % 9) < 2:
                img.blend(x, y, GOLD_DIM, 0.22)
    runes(img, 4, 4, 60, 28, GOLD_HI, seed=5, k=0.85)
    edge_trim(img, GOLD_DIM, thick=1)
    write_png(os.path.join(out, "vendas_runicas.png"), img.w, img.h, img.px)


def stained_glass(out, name, seed, palette, animated_frames=1):
    """Shared elytra art: organic cloud panes with contour leading, optionally
    N phase-shifted frames (the animation drifts the cloud field)."""
    for f in range(animated_frames):
        img = Img(64, 32)
        img.vgrad(palette[0], palette[min(1, len(palette) - 1)])
        phase = f / max(1, animated_frames) * math.tau
        drift_x, drift_y = math.cos(phase) * 1.2, math.sin(phase) * 1.2
        for y in range(img.h):
            for x in range(img.w):
                # Two large-scale clouds + a finer one -> organic panes.
                # Rotated noise domain: organic contours instead of bands.
                rx, ry = x * 0.82 + y * 0.55, y * 0.82 - x * 0.4
                n = fbm(rx * 0.075 + drift_x, ry * 0.075 + drift_y, 3, seed)
                m = fbm(rx * 0.17 - drift_y, ry * 0.17 + drift_x, 2, seed + 71)
                tone = min(1.0, max(0.0, n * 0.75 + m * 0.35 - 0.08))
                band = tone * (len(palette) - 1)
                idx = min(len(palette) - 1, int(band))
                frac = band - idx
                c0 = palette[idx]
                c1 = palette[min(len(palette) - 1, idx + 1)]
                col = tuple(int(c0[i] * (1 - frac) + c1[i] * frac) for i in range(4))
                img.blend(x, y, col, 0.85)
                # Contour leading between panes (irregular, fbm-driven).
                if frac < 0.06 and tone > 0.12:
                    img.blend(x, y, GOLD_DIM, 0.45)
        grain(img, seed=seed + f, strength=5)
        edge_trim(img, GOLD, thick=1)
        suffix = "" if animated_frames == 1 else "_%d" % f
        write_png(os.path.join(out, "%s%s.png" % (name, suffix)), img.w, img.h, img.px)


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
        os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
        "branding-mod", "src", "main", "resources", "assets", "espectral-menu",
        "textures", "cosmetics")
    os.makedirs(out, exist_ok=True)
    mantle(out)
    revenant(out)
    revenant_hood(out)
    tatters(out)
    wings(out)
    scarf(out)
    scarf_tail(out)
    ears(out)
    wraps(out)
    stained_glass(out, "elitros_vidriera", 101,
                  [(38, 30, 22, 255), (120, 84, 40, 255), (198, 152, 66, 255), (255, 214, 130, 255)])
    stained_glass(out, "elitros_nebulosa", 211,
                  [(22, 22, 40, 255), (52, 46, 96, 255), (120, 96, 168, 255), (232, 182, 76, 255)],
                  animated_frames=8)
    stained_glass(out, "elitros_ceniza", 307,
                  [(20, 20, 26, 255), (52, 48, 52, 255), (126, 108, 92, 255), (214, 150, 74, 255)],
                  animated_frames=8)
    files = sorted(f for f in os.listdir(out) if f.endswith(".png"))
    print("wrote %d textures to %s" % (len(files), out))
    for f in files:
        print("  ", f)


if __name__ == "__main__":
    main()
