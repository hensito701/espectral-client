#!/usr/bin/env python3
"""Side-by-side + diff of a reference screenshot and a Slint screenshot.

    python3 native/app/scripts/compare.py REF.png OURS.png OUT.png

OUT.png = [reference | slint | amplified abs diff]. Prints the mean abs
difference per channel (0-255) as a rough similarity number. Needs Pillow.
"""
import sys
from PIL import Image, ImageChops

ref, ours, out = sys.argv[1:4]
a = Image.open(ref).convert('RGB')
b = Image.open(ours).convert('RGB').resize(a.size)
d = ImageChops.difference(a, b)
stat = [sum(i * n for i, n in enumerate(d.histogram()[c * 256:(c + 1) * 256])) / (a.size[0] * a.size[1]) for c in range(3)]
d = d.point(lambda v: min(255, v * 4))
w, h = a.size
canvas = Image.new('RGB', (w * 3, h))
canvas.paste(a, (0, 0))
canvas.paste(b, (w, 0))
canvas.paste(d, (w * 2, 0))
canvas.save(out)
print(f'mean abs diff r={stat[0]:.1f} g={stat[1]:.1f} b={stat[2]:.1f} -> {out}')
