"""The ``--preview`` software renderer.

Contract: ``docs/cosmetics-pipeline.md`` ("Adding an item" step 3) — the build
renders each model item on a plain default player so a human can judge it
before it ships, and so an author never has to launch the game to see whether
the ears point the right way.

Orthographic, one texture sample per model pixel, painter's algorithm with
back-face culling, :data:`SCALE` screen pixels per model pixel. Columns are the
five review angles (front, back, left, right, three-quarter front); rows are the
rest pose plus one row per driver extreme that actually changes the shape.
"""

from __future__ import annotations

import math

from PIL import Image, ImageChops, ImageDraw, ImageFont

from geometry import (
    ATTACHMENT_PIVOTS_BEDROCK,
    Geometry,
    bedrock_to_java_cube,
    bedrock_to_java_point,
    bone_transforms,
    mat_vec,
)

#: Screen pixels per model pixel.
SCALE = 8
#: Label strip height above each tile, in screen pixels.
LABEL_HEIGHT = 13
#: Gap between tiles and around the sheet.
GAP = 6
#: Plain background of a tile, the label strip, and the sheet itself.
BACKGROUND = (232, 235, 240, 255)
LABEL_BACKGROUND = (205, 211, 220, 255)
LABEL_INK = (38, 42, 52, 255)
SHEET_BACKGROUND = (250, 250, 252, 255)
#: Plain default player: skin, shirt, trousers.
SKIN = (222, 170, 132)
SHIRT = (74, 108, 168)
TROUSERS = (52, 60, 88)
EYE_WHITE = (240, 240, 240)
PUPIL = (73, 62, 140)

#: Outward direction of each face of a vanilla box. ``ModelPart.Cube`` builds
#: DOWN, UP, WEST, NORTH, EAST, SOUTH in that order; model space has +Y down.
FACE_DIRECTIONS = (
    (0.0, 1.0, 0.0),
    (0.0, -1.0, 0.0),
    (-1.0, 0.0, 0.0),
    (0.0, 0.0, -1.0),
    (1.0, 0.0, 0.0),
    (0.0, 0.0, 1.0),
)


class Quad:
    """One flat, textured (or flat-coloured) polygon ready for projection."""

    __slots__ = ("points", "normal", "uvs", "color", "texture")

    def __init__(self, points, normal, uvs=None, color=None, texture=None):
        self.points = points
        self.normal = normal
        self.uvs = uvs
        self.color = color
        self.texture = texture


# ---------------------------------------------------------------------------
# Cubes -> quads (vanilla ModelPart unwrap)
# ---------------------------------------------------------------------------


def cube_faces(java_min, size, inflate=0.0, mirror=False):
    """Six faces of a vanilla box as ``(points, uv_rect)``.

    Follows ``ModelPart.Cube`` quad for quad: the box unwrap reserves
    ``2*(d+w)`` by ``d+h`` texels, ``inflate`` grows the geometry but not the
    uv rect, and ``mirror`` swaps the two x corners and reverses each quad,
    which ends up mirroring the texture across the box.
    """

    x0, y0, z0 = java_min
    dx, dy, dz = size

    ax, ay, az = x0 - inflate, y0 - inflate, z0 - inflate
    bx, by, bz = x0 + dx + inflate, y0 + dy + inflate, z0 + dz + inflate
    if mirror:
        ax, bx = bx, ax

    # Corner names follow ModelPart.Cube: `v` is the first x extreme, `v7` the
    # other; y/z stay in min/max order (+Y down).
    v = (bx, ay, az)
    v1 = (bx, by, az)
    v2 = (ax, by, az)
    v3 = (ax, ay, bz)
    v4 = (bx, ay, bz)
    v5 = (bx, by, bz)
    v6 = (ax, by, bz)
    v7 = (ax, ay, az)

    # Unwrap bands relative to the cube's uv origin.
    u = (0.0, dz, dz + dx, dz + 2 * dx, 2 * dz + dx, 2 * dz + 2 * dx)
    vv = (0.0, dz, dz + dy)

    faces = (
        ([v4, v3, v7, v], (u[1], vv[0], u[2], vv[1])),  # down
        ([v1, v2, v6, v5], (u[2], vv[1], u[3], vv[0])),  # up
        ([v7, v3, v6, v2], (u[0], vv[1], u[1], vv[2])),  # west
        ([v, v7, v2, v1], (u[1], vv[1], u[2], vv[2])),  # north
        ([v4, v, v1, v5], (u[2], vv[1], u[4], vv[2])),  # east
        ([v3, v4, v5, v6], (u[4], vv[1], u[5], vv[2])),  # south
    )
    if mirror:
        faces = tuple((list(reversed(points)), rect) for points, rect in faces)
    return list(faces)


def face_uvs(uv_rect, mirror: bool):
    """Per-corner UVs for a face, in ``ModelPart.Polygon``'s order."""

    u1, v1, u2, v2 = uv_rect
    corners = [(u2, v1), (u1, v1), (u1, v2), (u2, v2)]
    return list(reversed(corners)) if mirror else corners


def _normal(points):
    ax, ay, az = points[1][0] - points[0][0], points[1][1] - points[0][1], points[1][2] - points[0][2]
    bx, by, bz = points[2][0] - points[0][0], points[2][1] - points[0][1], points[2][2] - points[0][2]
    n = (ay * bz - az * by, az * bx - ax * bz, ax * by - ay * bx)
    length = math.sqrt(n[0] * n[0] + n[1] * n[1] + n[2] * n[2])
    if length < 1e-12:
        return None
    return (n[0] / length, n[1] / length, n[2] / length)


def _shade(color, normal):
    """Small per-face shade so a flat-coloured box still reads as a box."""

    if normal is None:
        return color
    light = (
        max(0.0, -normal[1]) * 0.55
        + max(0.0, -normal[2]) * 0.30
        + max(0.0, -normal[0]) * 0.15
    )
    factor = 0.72 + 0.36 * light
    return tuple(min(255, int(c * factor)) for c in color)


# ---------------------------------------------------------------------------
# Scene assembly
# ---------------------------------------------------------------------------


def player_boxes():
    """The plain default player, in vanilla model space (contract's frame)."""

    return (
        ("head", (-4.0, -8.0, -4.0), (8.0, 8.0, 8.0), SKIN),
        # Steve-like eyes on the face (y -4..-3), so face-covering items can
        # be judged against where the eyes are.
        ("right_eye", (-3.0, -4.0, -4.05), (2.0, 1.0, 0.04), EYE_WHITE),
        ("left_eye", (1.0, -4.0, -4.05), (2.0, 1.0, 0.04), EYE_WHITE),
        ("right_pupil", (-2.0, -4.0, -4.1), (1.0, 1.0, 0.04), PUPIL),
        ("left_pupil", (1.0, -4.0, -4.1), (1.0, 1.0, 0.04), PUPIL),
        ("body", (-4.0, 0.0, -2.0), (8.0, 12.0, 4.0), SHIRT),
        ("right_arm", (-8.0, 0.0, -2.0), (4.0, 12.0, 4.0), SKIN),
        ("left_arm", (4.0, 0.0, -2.0), (4.0, 12.0, 4.0), SKIN),
        ("right_leg", (-3.9, 12.0, -2.0), (4.0, 12.0, 4.0), TROUSERS),
        ("left_leg", (-0.1, 12.0, -2.0), (4.0, 12.0, 4.0), TROUSERS),
    )


def player_quads():
    quads = []
    for _name, java_min, size, color in player_boxes():
        for face_index, (vertices, _uv) in enumerate(cube_faces(java_min, size)):
            quads.append(Quad(vertices, FACE_DIRECTIONS[face_index], color=color))
    return quads


def item_quads(geometry: Geometry, pose, texture_image):
    """The item's faces for ``pose``, already in the attachment part's frame."""

    part = geometry.attachment
    transforms = bone_transforms(geometry, pose)
    quads: list[Quad] = []
    for bone in geometry.ordered_bones():
        if bone.name == part or not bone.cubes:
            continue
        matrix, origin = transforms[bone.name]
        bone_pivot = bedrock_to_java_point(bone.pivot)
        for cube in bone.cubes:
            java_min = bedrock_to_java_cube(cube.origin, cube.size)
            local_min = (
                java_min[0] - bone_pivot[0],
                java_min[1] - bone_pivot[1],
                java_min[2] - bone_pivot[2],
            )
            for face_index, (vertices, uv_rect) in enumerate(
                cube_faces(local_min, cube.size, cube.inflate, cube.mirror)
            ):
                direction = FACE_DIRECTIONS[face_index]
                if cube.mirror:
                    direction = (-direction[0], direction[1], direction[2])
                world = []
                for point in vertices:
                    moved = mat_vec(matrix, point)
                    world.append((origin[0] + moved[0], origin[1] + moved[1], origin[2] + moved[2]))
                # The bone matrix only rotates, so the normal rides along.
                normal = mat_vec(matrix, direction)
                uvs = [
                    (cube.uv[0] + u, cube.uv[1] + v)
                    for u, v in face_uvs(uv_rect, cube.mirror)
                ]
                quads.append(Quad(world, normal, uvs=uvs, texture=texture_image))
    return quads


def model_bounds(quads):
    """``(min_x, max_x, min_y, max_y, min_z, max_z)`` over every quad corner."""

    xs = [p[0] for quad in quads for p in quad.points]
    ys = [p[1] for quad in quads for p in quad.points]
    zs = [p[2] for quad in quads for p in quad.points]
    return min(xs), max(xs), min(ys), max(ys), min(zs), max(zs)


# ---------------------------------------------------------------------------
# Cameras
# ---------------------------------------------------------------------------

#: Review angles, as the direction from the model to the camera. The wearer
#: faces -Z, so the front camera sits on the -Z side; the wearer's right is -X,
#: which puts it on the viewer's left in the front view.
VIEWS = (
    ("front", (0.0, 0.0, -1.0)),
    ("back", (0.0, 0.0, 1.0)),
    ("left", (1.0, 0.0, 0.0)),
    ("right", (-1.0, 0.0, 0.0)),
    ("3/4 front", (0.70710678, 0.0, -0.70710678)),
)


def camera(eye_direction):
    """``(right, up, forward)`` screen basis for a camera along ``eye_direction``."""

    length = math.sqrt(sum(c * c for c in eye_direction))
    ex, ey, ez = (c / length for c in eye_direction)
    forward = (-ex, -ey, -ez)
    up = (0.0, -1.0, 0.0)  # model +Y is down, so screen up is -Y
    right = (
        forward[1] * up[2] - forward[2] * up[1],
        forward[2] * up[0] - forward[0] * up[2],
        forward[0] * up[1] - forward[1] * up[0],
    )
    up = (
        right[1] * forward[2] - right[2] * forward[1],
        right[2] * forward[0] - right[0] * forward[2],
        right[0] * forward[1] - right[1] * forward[0],
    )
    return right, up, forward


# ---------------------------------------------------------------------------
# Rasteriser
# ---------------------------------------------------------------------------


def _solve_affine(source, target):
    """Coefficients mapping output ``(x, y)`` to input ``(u, v)`` from three points."""

    (x0, y0), (x1, y1), (x2, y2) = source
    (u0, v0), (u1, v1), (u2, v2) = target
    det = (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0)
    if abs(det) < 1e-9:
        return None
    a = ((u1 - u0) * (y2 - y0) - (u2 - u0) * (y1 - y0)) / det
    b = ((u2 - u0) * (x1 - x0) - (u1 - u0) * (x2 - x0)) / det
    d = ((v1 - v0) * (y2 - y0) - (v2 - v0) * (y1 - y0)) / det
    e = ((v2 - v0) * (x1 - x0) - (v1 - v0) * (x2 - x0)) / det
    return (a, b, u0 - a * x0 - b * y0, d, e, v0 - d * x0 - e * y0)


def _project(points, right, up, forward, cx, cy):
    """Project to screen pixels; the third member is depth along ``forward``."""

    out = []
    for px, py, pz in points:
        sx = cx + (px * right[0] + py * right[1] + pz * right[2]) * SCALE
        sy = cy - (px * up[0] + py * up[1] + pz * up[2]) * SCALE
        out.append((sx, sy, px * forward[0] + py * forward[1] + pz * forward[2]))
    return out


def render_view(canvas, quads, eye):
    """Draw ``quads`` onto ``canvas`` (an RGBA image) from ``eye``'s angle."""

    right, up, forward = camera(eye)
    width, height = canvas.size
    cx = width / 2.0
    cy = height / 2.0
    draw = ImageDraw.Draw(canvas)

    prepared = []
    for quad in quads:
        normal = quad.normal
        if normal is None:
            continue
        if normal[0] * forward[0] + normal[1] * forward[1] + normal[2] * forward[2] >= -1e-6:
            continue  # faces away from the camera
        screen = _project(quad.points, right, up, forward, cx, cy)
        depth = sum(p[2] for p in screen) / len(screen)
        prepared.append((depth, quad, [(p[0], p[1]) for p in screen]))
    prepared.sort(key=lambda entry: entry[0], reverse=True)

    for _depth, quad, polygon in prepared:
        if quad.texture is not None and quad.uvs is not None:
            _draw_textured(canvas, quad, polygon)
        elif quad.color is not None:
            colour = _shade(quad.color, quad.normal)
            draw.polygon(polygon, fill=(colour[0], colour[1], colour[2], 255))


def _draw_textured(canvas, quad, polygon):
    source = quad.texture
    if source is None:
        return
    lo_x = max(0, int(math.floor(min(p[0] for p in polygon))))
    hi_x = min(canvas.size[0], int(math.ceil(max(p[0] for p in polygon))))
    lo_y = max(0, int(math.floor(min(p[1] for p in polygon))))
    hi_y = min(canvas.size[1], int(math.ceil(max(p[1] for p in polygon))))
    if hi_x <= lo_x or hi_y <= lo_y:
        return
    coeffs = _solve_affine(
        [(polygon[0][0], polygon[0][1]), (polygon[1][0], polygon[1][1]), (polygon[2][0], polygon[2][1])],
        [(quad.uvs[0][0], quad.uvs[0][1]), (quad.uvs[1][0], quad.uvs[1][1]), (quad.uvs[2][0], quad.uvs[2][1])],
    )
    if coeffs is None:
        return
    a, b, c, d, e, f = coeffs
    # The crop starts at (lo_x, lo_y) in canvas space; shift the mapping so
    # output pixel (0, 0) of the crop lands on the same texel.
    shifted = (a, b, c + a * lo_x + b * lo_y, d, e, f + d * lo_x + e * lo_y)
    size = (hi_x - lo_x, hi_y - lo_y)
    warped = source.transform(size, Image.AFFINE, shifted, resample=Image.NEAREST)

    mask = Image.new("L", size, 0)
    ImageDraw.Draw(mask).polygon([(p[0] - lo_x, p[1] - lo_y) for p in polygon], fill=255)
    alpha = warped.getchannel("A")
    if alpha.getextrema() != (255, 255):
        mask = ImageChops.multiply(mask, alpha)
    if mask.getextrema() == (0, 0):
        return
    canvas.paste(warped, (lo_x, lo_y), mask)


# ---------------------------------------------------------------------------
# The sheet
# ---------------------------------------------------------------------------

def _label_font():
    """A system font with Spanish glyphs (ñ, á…); Pillow's bundled default lacks them."""
    for name in ("arial.ttf", "DejaVuSans.ttf", "LiberationSans-Regular.ttf"):
        try:
            return ImageFont.truetype(name, 12)
        except OSError:
            continue
    return ImageFont.load_default()


def render_sheet(item: dict, geometry: Geometry, texture_image, rows) -> Image.Image:
    """Render the review sheet for one item.

    ``rows`` is ``[(label, pose), …]``; the first entry is the rest pose.
    """

    font = _label_font()
    player = player_quads()
    poses = [item_quads(geometry, pose, texture_image) for _label, pose in rows]

    bounds = [model_bounds(quads) for quads in poses] + [model_bounds(player)]
    min_x = min(b[0] for b in bounds) - 0.6
    max_x = max(b[1] for b in bounds) + 0.6
    min_y = min(b[2] for b in bounds) - 0.6
    max_y = max(b[3] for b in bounds) + 0.6
    min_z = min(b[4] for b in bounds) - 0.6
    max_z = max(b[5] for b in bounds) + 0.6

    half_span = max(-min_x, max_x, -min_z, max_z)
    view_w = int(math.ceil(2 * half_span * SCALE))
    view_h = int(math.ceil((max_y - min_y) * SCALE))
    view_w += view_w % 2
    view_h += view_h % 2

    tile_w, tile_h = view_w, view_h + LABEL_HEIGHT
    sheet_w = GAP + len(VIEWS) * (tile_w + GAP)
    sheet_h = GAP + LABEL_HEIGHT + len(rows) * (tile_h + GAP)
    sheet = Image.new("RGBA", (sheet_w, sheet_h), SHEET_BACKGROUND)

    # ASCII separators only: the bundled default font has no em dash.
    title = "%s | %s / %s" % (item["id"], item["name"]["es"], item["name"]["en"])
    ImageDraw.Draw(sheet).text((GAP, 1), title, fill=LABEL_INK, font=font)

    for row_index, (row_label, _pose) in enumerate(rows):
        quads = poses[row_index] + player
        for column_index, (view_label, eye) in enumerate(VIEWS):
            view = Image.new("RGBA", (view_w, view_h), BACKGROUND)
            render_view(view, quads, eye)
            tile = Image.new("RGBA", (tile_w, tile_h), BACKGROUND)
            strip = ImageDraw.Draw(tile)
            strip.rectangle([0, 0, tile_w - 1, LABEL_HEIGHT - 1], fill=LABEL_BACKGROUND)
            strip.text((3, 0), "%s · %s" % (view_label, row_label), fill=LABEL_INK, font=font)
            tile.paste(view, (0, LABEL_HEIGHT))
            sheet.paste(
                tile,
                (GAP + column_index * (tile_w + GAP), GAP + LABEL_HEIGHT + row_index * (tile_h + GAP)),
            )
    return sheet


__all__ = [
    "FACE_DIRECTIONS",
    "Quad",
    "SCALE",
    "VIEWS",
    "camera",
    "cube_faces",
    "face_uvs",
    "item_quads",
    "model_bounds",
    "player_quads",
    "render_sheet",
]
