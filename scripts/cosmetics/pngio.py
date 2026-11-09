"""PNG and ``texture.txt`` I/O for the cosmetics build.

Reading goes through Pillow (fine for any RGBA PNG); writing uses the standard
library only (``zlib`` + ``struct``) so a build on any machine produces the same
bytes for the same pixels, which is what keeps the committed files stable.

Contract: ``docs/cosmetics-pipeline.md`` ("Texture").
"""

from __future__ import annotations

import struct
import sys
import zlib
from pathlib import Path

_HERE = str(Path(__file__).resolve().parent)
if _HERE not in sys.path:
    sys.path.insert(0, _HERE)

RGBA = tuple[int, int, int, int]

#: The contract's minimum opacity: a texel is either fully transparent or at
#: least this opaque, so no cosmetic can render as a ghost.
MIN_ALPHA = 200

#: Palette key reserved for "no pixel here".
TRANSPARENT_KEY = "."


class TextureError(ValueError):
    """A texture source is malformed; the message is build output."""


# ---------------------------------------------------------------------------
# PNG
# ---------------------------------------------------------------------------


def write_png(path: Path, width: int, height: int, rows: list[list[RGBA]]) -> bytes:
    """Write an 8-bit RGBA PNG deterministically and return its bytes."""

    png = png_bytes(width, height, rows)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(png)
    return png


def png_bytes(width: int, height: int, rows: list[list[RGBA]]) -> bytes:
    """Encode an 8-bit RGBA PNG (filter 0, zlib level 9) with no metadata."""

    raw = bytearray()
    for row in rows:
        raw.append(0)  # filter: none
        for r, g, b, a in row:
            raw += bytes((r & 0xFF, g & 0xFF, b & 0xFF, a & 0xFF))

    def chunk(tag: bytes, data: bytes) -> bytes:
        crc = zlib.crc32(tag + data) & 0xFFFFFFFF
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", crc)

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
    png += chunk(b"IEND", b"")
    return png


def read_png(path: Path) -> tuple[int, int, list[list[RGBA]]]:
    """Read an RGBA PNG as ``(width, height, rows)``."""

    from PIL import Image

    with Image.open(path) as image:
        rgba = image.convert("RGBA")
        width, height = rgba.size
        data = rgba.tobytes()
    rows: list[list[RGBA]] = []
    stride = width * 4
    for y in range(height):
        base = y * stride
        rows.append(
            [
                (data[base + 4 * x], data[base + 4 * x + 1], data[base + 4 * x + 2], data[base + 4 * x + 3])
                for x in range(width)
            ]
        )
    return width, height, rows


# ---------------------------------------------------------------------------
# texture.txt
# ---------------------------------------------------------------------------


def parse_colour(text: str) -> RGBA:
    """``#rrggbb`` (opaque) or ``#rrggbbaa`` -> RGBA, or :class:`TextureError`."""

    if not text.startswith("#"):
        raise TextureError(f"colour '{text}' must start with '#'")
    digits = text[1:]
    if len(digits) not in (6, 8):
        raise TextureError(f"colour '{text}' must be #rrggbb or #rrggbbaa")
    try:
        values = [int(digits[i : i + 2], 16) for i in range(0, len(digits), 2)]
    except ValueError:
        raise TextureError(f"colour '{text}' is not hexadecimal") from None
    if len(values) == 3:
        values.append(255)
    return (values[0], values[1], values[2], values[3])


def parse_texture_txt(text: str) -> tuple[int, int, list[list[RGBA]]]:
    """Parse the text pixel-art format and return the pixels it paints.

    Layout::

        size 64 32
        palette
        . transparent
        R #b3262e
        rows
        RRRR...
    """

    lines = text.splitlines()
    if not lines:
        raise TextureError("empty texture.txt")

    width = height = None
    index = 0
    while index < len(lines):
        line = lines[index].strip()
        index += 1
        if not line:
            continue
        parts = line.split()
        if parts[0] != "size" or len(parts) != 3:
            raise TextureError(f"expected 'size <width> <height>', found '{line}'")
        try:
            width, height = int(parts[1]), int(parts[2])
        except ValueError:
            raise TextureError(f"'size' needs two integers, found '{line}'") from None
        if width <= 0 or height <= 0:
            raise TextureError(f"'size' must be positive, found '{line}'")
        break
    if width is None or height is None:
        raise TextureError("missing 'size <width> <height>' header")

    # ---- palette ---------------------------------------------------------
    palette: dict[str, RGBA | None] = {TRANSPARENT_KEY: None}
    declared: set[str] = set()
    while index < len(lines) and not lines[index].strip():
        index += 1
    if index >= len(lines) or lines[index].strip() != "palette":
        raise TextureError("missing 'palette' section")
    index += 1
    while index < len(lines):
        line = lines[index].strip()
        index += 1
        if not line:
            continue
        if line == "rows":
            break
        parts = line.split()
        if len(parts) != 2:
            raise TextureError(f"palette entry '{line}' must be '<key> <colour>'")
        key, colour = parts
        if len(key) != 1 or not key.isprintable() or key.isspace():
            raise TextureError(f"palette key '{key}' must be a single printable non-space character")
        if key in declared:
            raise TextureError(f"palette key '{key}' is declared twice")
        declared.add(key)
        if colour == "transparent":
            if key != TRANSPARENT_KEY:
                raise TextureError(
                    f"palette key '{key}': only '.' may be transparent ('{key} transparent')"
                )
            palette[key] = None
            continue
        if key == TRANSPARENT_KEY:
            raise TextureError("palette key '.' is reserved for transparent and cannot be painted")
        palette[key] = parse_colour(colour)
    else:
        raise TextureError("missing 'rows' section")

    # ---- rows ------------------------------------------------------------
    rows: list[list[RGBA]] = []
    for number, line in enumerate(lines[index:], start=1):
        if not line:
            continue
        if line.startswith("#") or line.startswith("//"):
            continue
        if len(line) != width:
            raise TextureError(f"row {number} has {len(line)} characters, expected {width}")
        row: list[RGBA] = []
        for column, key in enumerate(line):
            if key not in palette:
                raise TextureError(
                    f"row {number} column {column} uses undefined palette key '{key}'"
                )
            colour = palette[key]
            row.append((0, 0, 0, 0) if colour is None else colour)
        rows.append(row)
    if len(rows) != height:
        raise TextureError(f"expected {height} rows of pixels, found {len(rows)}")
    return width, height, rows


# ---------------------------------------------------------------------------
# Shared checks
# ---------------------------------------------------------------------------


def alpha_rule_violations(width: int, height: int, rows: list[list[RGBA]], limit: int = 4) -> list[str]:
    """Texels below :data:`MIN_ALPHA` (and not fully transparent), first few named."""

    offenders: list[str] = []
    count = 0
    for y in range(height):
        for x in range(width):
            alpha = rows[y][x][3]
            if 0 < alpha < MIN_ALPHA:
                count += 1
                if len(offenders) < limit:
                    offenders.append(f"({x},{y}) alpha {alpha}")
    if not count:
        return []
    extra = "" if count <= len(offenders) else f" (+{count - len(offenders)} more)"
    return [f"{count} texel(s) below alpha {MIN_ALPHA}: " + ", ".join(offenders) + extra]
