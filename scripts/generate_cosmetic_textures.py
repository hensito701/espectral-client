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
    """One-shoulder mantle, 64x64 (u across the back, v from shoulder to hem).

    Reads as fabric rather than a slab because of three shading layers: broad
    vertical folds (a wavy displacement, so no fold is ruler-straight) with a
    real dark-to-light range, a cross-body light falloff from the clasp
    shoulder, and a woven twill grain. The hem is a gold-mist fade whose alpha
    never drops below the cosmetic minimum, so the cloth dissolves into light
    instead of ending on a hard line.
    """
    img = Img(64, 64)
    img.vgrad(DARK2, DARK)
    FOLD_HI = (58, 60, 80, 255)
    FOLD_LO = (8, 9, 14, 255)
    for y in range(img.h):
        for x in range(img.w):
            # Three fold systems: one slow, one medium, one tight.
            primary = math.sin(x * 0.42 + y * 0.05 + 0.6)
            secondary = math.sin(x * 0.95 - y * 0.09 + 2.1) * 0.7
            tight = math.sin(x * 1.9 + y * 0.15) * 0.35
            fold = primary + secondary + tight
            # Soft-clip so the crests stay rounded cloth, not posterised bands.
            if fold > 0:
                img.blend(x, y, FOLD_HI, min(1.0, fold * 0.42))
            else:
                img.blend(x, y, FOLD_LO, min(1.0, -fold * 0.46))
            # Cross-body light: brightest at the clasp shoulder (top-left).
            light = 1.0 - 0.6 * math.hypot((x - 4) / img.w, (y - 2) / img.h)
            img.blend(x, y, (52, 54, 72, 255), max(0.0, light) * 0.30)
    grain(img, seed=3, strength=10, scale=0.75)
    # Woven twill: one-pixel diagonal ribs, weak enough to read as texture.
    for y in range(img.h):
        for x in range(img.w):
            if (x + y) % 4 == 0:
                img.blend(x, y, SHADE, 0.20)
            elif (x - y) % 6 == 0:
                img.blend(x, y, (10, 11, 16, 255), 0.14)
    # Gold piping down both selvedges only (u = 0 / u = 1 are the sheet's sides).
    for y in range(48):
        for x in range(img.w):
            d = min(x, img.w - 1 - x)
            if d < 2:
                img.blend(x, y, GOLD, (2 - d) / 2 * 0.75)
    # The clasp: a small round gold boss on the clasp shoulder, top-left.
    img.glow(6, 8, 7, GOLD_HI, 0.42)
    for y in range(5, 12):
        for x in range(3, 10):
            d = math.hypot((x - 6.5) / 3.4, (y - 8.5) / 3.4)
            if d <= 1.0:
                k = 1.0 - d * 0.75
                img.blend(x, y, GOLD, 0.55 + k * 0.4)
                if d < 0.55:
                    img.blend(x, y, (255, 246, 222, 255), 0.75)
    # Short strap running back over the shoulder from the boss.
    for i in range(9):
        img.blend(9 + i, 7 + i // 3, GOLD_DIM, 0.45 - i * 0.035)
    # Hem: gold mist rising through the last fifth of the sheet.
    hem_from = 50
    for x in range(img.w):
        wob = 1.8 * math.sin(x * 0.42) + 1.3 * math.sin(x * 0.17 + 1.3)
        for y in range(hem_from, img.h):
            t = (y - hem_from) / (img.h - 1 - hem_from)
            edge = max(0.0, 1.0 - (y - hem_from - 2 - wob) / 3.0)
            k = min(1.0, t * 0.9 + edge * 0.55)
            img.blend(x, y, GOLD, 0.12 + k * 0.5)
            if t > 0.55:
                img.blend(x, y, GOLD_HI, (t - 0.55) * 1.4)
    # Let the very hem dissolve: alpha ramps down but never below the minimum
    # cosmetic opacity (the renderer clamps too; this keeps the art honest).
    for x in range(img.w):
        for y in range(img.h - 9, img.h):
            r, g, b, a = img.get(x, y)
            fade = (y - (img.h - 10)) / 9.0
            img.set(x, y, (r, g, b, max(200, int(a - fade * 55))))
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


# Ear atlas layout (must match CosmeticModels UV_* constants). Vanilla unwraps
# a box as `2*(d+w)` wide by `d+h` tall, so each island is painted implicitly:
#   seg0 (3w x 2h x 3d) -> 12 x 5 at (0, 0)
#   seg1 (2w x 2h x 2d) ->  8 x 4 at (16, 0)
#   tip  (1w x 1.4h x 1d) -> 4 x 2.4 at (28, 0)
#   inner panel          -> 4 x 2 at (36, 0) and (36, 4)
#   tuft cuff (feline)   -> 13 x 3.5 at (48, 0)
EAR_MAIN = (0, 0, 32, 8)      # seg0 + seg1 outer fur
EAR_TIP = (28, 0, 32, 4)      # dark tip cap
EAR_INNER = (36, 0, 42, 8)    # inner-ear panel
EAR_TUFT = (48, 0, 62, 4)     # base cuff


def ear_sheet(out, name, outer_top, outer_bottom, tip, inner, seed, streaks):
    """Paints one ear atlas: a shaded fur body, a dark tip, a bright inner ear
    and a fur cuff. Both ear items share the layout, only the palette differs."""
    img = Img(64, 32)
    # Fur body: vertical gradient (light at the base, dark toward the tip) plus
    # grain so it does not read as flat plastic.
    x0, y0, x1, y1 = EAR_MAIN
    for y in range(y0, y1):
        t = y / max(1, y1 - 1)
        col = tuple(int(outer_top[i] * (1 - t) + outer_bottom[i] * t) for i in range(4))
        for x in range(x0, x1):
            img.set(x, y, col)
    grain(img, seed=seed, strength=12, scale=0.9)
    # Fur streaks running base -> tip.
    for i in range(streaks):
        sx = (i * 7 + 1) % (x1 - x0)
        for y in range(y0, y1):
            img.blend(x0 + (sx + y // 3) % (x1 - x0), y, outer_top, 0.16)
    # Tip: near black, slightly cooler toward the very point.
    tx0, ty0, tx1, ty1 = EAR_TIP
    for y in range(ty0, ty1):
        t = (y - ty0) / max(1, ty1 - ty0 - 1)
        col = tuple(int(tip[i] * (1 - t * 0.35)) for i in range(4))
        for x in range(tx0, tx1):
            img.set(x, y, col)
    # Inner ear: bright fur/skin with a soft centre glow and a darker rim, so
    # it reads as a hollow even at 4 px wide.
    ix0, iy0, ix1, iy1 = EAR_INNER
    for y in range(iy0, iy1):
        for x in range(ix0, ix1):
            img.set(x, y, inner)
    img.glow(ix0 + (ix1 - ix0) / 2 - 0.5, iy0 + 2, 3.0, (255, 255, 255, 255), 0.25)
    edge_trim_rect(img, (ix0, iy0, ix1, iy1), (0, 0, 0, 255), 0.45)
    # Tuft cuff shares the outer fur so it blends into the ear base.
    gx0, gy0, gx1, gy1 = EAR_TUFT
    for y in range(gy0, gy1):
        for x in range(gx0, gx1):
            img.set(x, y, outer_bottom)
    for y in range(gy0, gy1):
        for x in range(gx0, gx1):
            img.blend(x, y, outer_top, 0.12 if (x + y) % 3 else 0.3)
    write_png(os.path.join(out, name), img.w, img.h, img.px)


def edge_trim_rect(img, rect, rgba, k):
    x0, y0, x1, y1 = rect
    for y in range(y0, y1):
        for x in range(x0, x1):
            if x in (x0, x1 - 1) or y in (y0, y1 - 1):
                img.blend(x, y, rgba, k)


def ears(out):
    """Vulpine: orange-red fur, cream inner fur, dark tips."""
    ear_sheet(out, "orejas_vulpinas.png",
              (214, 118, 58, 255), (150, 66, 34, 255),
              (30, 22, 22, 255), (246, 232, 208, 255),
              seed=37, streaks=9)
    """Feline: dark charcoal fur, pink inner ear, near-black tips."""
    ear_sheet(out, "orejas_felinas.png",
              (74, 72, 84, 255), (34, 32, 42, 255),
              (18, 16, 22, 255), (226, 146, 158, 255),
              seed=53, streaks=7)


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
