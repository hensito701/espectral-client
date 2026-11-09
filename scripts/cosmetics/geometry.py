"""Bedrock geometry: parsing, validation, conversion and footprint math.

Contract: ``docs/cosmetics-pipeline.md`` ("model.geo.json", "Conversion to the
game's model space", "Checks" rules 2 and 4).

Everything here is Minecraft-free so the build (and its tests) can run without
the game: a geometry is read as plain numbers, converted to the vanilla model
space the mod's loader uses, and swept through every bone rotation to bound the
item's footprint.
"""

from __future__ import annotations

import math

#: Attachment bones: the vanilla player part each one rides on. Values are the
#: Bedrock (feet-origin, Y up) pivots the contract fixes for them.
ATTACHMENT_PIVOTS_BEDROCK: dict[str, tuple[float, float, float]] = {
    "head": (0.0, 24.0, 0.0),
    "body": (0.0, 24.0, 0.0),
    "right_arm": (-5.0, 22.0, 0.0),
    "left_arm": (5.0, 22.0, 0.0),
    "right_leg": (-1.9, 12.0, 0.0),
    "left_leg": (1.9, 12.0, 0.0),
}

#: Footprint limits in vanilla model space (pixels, +Y down, origin neck line).
HEAD_TOP_Y = -8.0
KNEE_Y = 18.0
MAX_ABS_X = 12.5

#: Whole-pixel cube sizes. Two items predate the rule: the ears were authored
#: in Java with fractional segments (a 1.4 px tip, 1.86 x 1.72 px inner panels,
#: a 3.6 x 0.8 px cuff) and their committed textures were painted for exactly
#: those unwraps, so snapping them would change owner-approved art. They keep
#: their exact geometry; every other item must use whole pixels.
LEGACY_FRACTIONAL_SIZES = frozenset({"orejas_vulpinas", "orejas_felinas"})

#: Keys a cube may carry; anything else (per-face UV, rotation, pivot) is out.
CUBE_KEYS = frozenset({"origin", "size", "uv", "inflate", "mirror"})
#: Keys a bone may carry.
BONE_KEYS = frozenset({"name", "parent", "pivot", "rotation", "cubes", "mirror"})
#: Standard-format features the subset does not support.
REJECTED_GEOMETRY_KEYS = ("locators", "poly_mesh", "texture_meshes", "binding")
#: Cube budget per item.
MAX_CUBES = 64


def bedrock_to_java_point(point: tuple[float, float, float]) -> tuple[float, float, float]:
    """Bedrock model point -> vanilla model space (X/Z identical, Y mirrored)."""

    x, y, z = point
    return (x, 24.0 - y, z)


def bedrock_to_java_cube(
    origin: tuple[float, float, float], size: tuple[float, float, float]
) -> tuple[float, float, float]:
    """Bedrock cube ``origin`` (min corner) -> vanilla space min corner."""

    x, y, z = origin
    sx, sy, sz = size
    return (x, 24.0 - (y + sy), z)


def rotation_matrix(rx: float, ry: float, rz: float) -> tuple[tuple[float, ...], ...]:
    """Vanilla ``ModelPart`` rotation: R = Rz * Ry * Rx, angles in degrees."""

    ax, ay, az = math.radians(rx), math.radians(ry), math.radians(rz)
    cx, sx = math.cos(ax), math.sin(ax)
    cy, sy = math.cos(ay), math.sin(ay)
    cz, sz = math.cos(az), math.sin(az)

    # Rx
    rx_m = ((1.0, 0.0, 0.0), (0.0, cx, -sx), (0.0, sx, cx))
    # Ry
    ry_m = ((cy, 0.0, sy), (0.0, 1.0, 0.0), (-sy, 0.0, cy))
    # Rz
    rz_m = ((cz, -sz, 0.0), (sz, cz, 0.0), (0.0, 0.0, 1.0))

    return mat_mul(rz_m, mat_mul(ry_m, rx_m))


def mat_mul(a, b):
    return tuple(
        tuple(sum(a[i][k] * b[k][j] for k in range(3)) for j in range(3)) for i in range(3)
    )


def mat_vec(m, v):
    return (
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    )


IDENTITY = ((1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, 1.0))


# ---------------------------------------------------------------------------
# Parsed geometry
# ---------------------------------------------------------------------------


class GeometryError(ValueError):
    """A geometry file is malformed; the message is build output."""


class Cube:
    """One box: Bedrock fields plus the vanilla numbers the runtime wants."""

    __slots__ = ("origin", "size", "uv", "inflate", "mirror")

    def __init__(self, origin, size, uv, inflate=0.0, mirror=False):
        self.origin = origin
        self.size = size
        self.uv = uv
        self.inflate = inflate
        self.mirror = mirror

    def uv_span(self) -> tuple[float, float]:
        """Box unwrap size: ``2*(d+w)`` wide by ``d+h`` tall."""

        width, height, depth = self.size
        return (2.0 * (depth + width), depth + height)


class Bone:
    __slots__ = ("name", "parent", "pivot", "rotation", "cubes", "motion", "children")

    def __init__(self, name, parent, pivot, rotation, cubes):
        self.name = name
        self.parent = parent
        self.pivot = pivot
        self.rotation = rotation
        self.cubes: list[Cube] = cubes
        self.motion: list[dict] = []
        self.children: list[str] = []


class Geometry:
    """One parsed ``model.geo.json``."""

    def __init__(self, identifier, texture_width, texture_height, bones):
        self.identifier = identifier
        self.texture_width = texture_width
        self.texture_height = texture_height
        self.bones: dict[str, Bone] = bones
        self.roots: list[str] = [b.name for b in bones.values() if b.parent is None]
        #: Items attach to exactly one player part, named by their only root.
        self.attachment = self.roots[0] if len(self.roots) == 1 else None

    def ordered_bones(self) -> list[Bone]:
        """Bones in parent-before-child order (authored order, then children)."""

        ordered: list[Bone] = []
        seen: set[str] = set()

        def visit(bone: Bone) -> None:
            if bone.name in seen:
                return
            seen.add(bone.name)
            ordered.append(bone)
            for child in bone.children:
                visit(self.bones[child])

        for root in self.roots:
            visit(self.bones[root])
        return ordered

    def cube_count(self) -> int:
        return sum(len(bone.cubes) for bone in self.bones.values())


def _number(value, what: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise GeometryError(f"{what} must be a number, found {value!r}")
    number = float(value)
    if not math.isfinite(number):
        raise GeometryError(f"{what} must be finite")
    return number


def _triple(value, what: str) -> tuple[float, float, float]:
    if not isinstance(value, list) or len(value) != 3:
        raise GeometryError(f"{what} must be an array of three numbers")
    return tuple(_number(v, f"{what}[{i}]") for i, v in enumerate(value))  # type: ignore[return-value]


def _pair(value, what: str) -> tuple[float, float]:
    if not isinstance(value, list) or len(value) != 2:
        raise GeometryError(f"{what} must be an array of two numbers")
    return (_number(value[0], f"{what}[0]"), _number(value[1], f"{what}[1]"))


def parse_cube(raw: object, where: str) -> Cube:
    if not isinstance(raw, dict):
        raise GeometryError(f"{where}: cube must be an object")
    unknown = set(raw) - CUBE_KEYS
    if unknown:
        raise GeometryError(
            f"{where}: unsupported cube key(s) {sorted(unknown)} — box UV only, no per-face UV"
        )
    if "origin" not in raw or "size" not in raw or "uv" not in raw:
        raise GeometryError(f"{where}: cube needs origin, size and uv")
    origin = _triple(raw["origin"], f"{where}.origin")
    size = _triple(raw["size"], f"{where}.size")
    if any(s <= 0.0 for s in size):
        raise GeometryError(f"{where}: size must be positive, found {list(size)}")
    if isinstance(raw["uv"], dict):
        raise GeometryError(f"{where}.uv: box UV only — per-face UV objects are not supported")
    uv = _pair(raw["uv"], f"{where}.uv")
    inflate = _number(raw.get("inflate", 0.0), f"{where}.inflate")
    if inflate < 0.0:
        raise GeometryError(f"{where}: inflate must not be negative")
    mirror = raw.get("mirror", False)
    if not isinstance(mirror, bool):
        raise GeometryError(f"{where}: mirror must be true or false")
    return Cube(origin, size, uv, inflate, mirror)


def parse_geometry(doc: object) -> Geometry:
    """Parse and structurally validate one Bedrock geometry document."""

    if not isinstance(doc, dict):
        raise GeometryError("model.geo.json must contain an object")
    if doc.get("format_version") != "1.12.0":
        raise GeometryError(
            f"format_version must be \"1.12.0\", found {doc.get('format_version')!r}"
        )
    geometries = doc.get("minecraft:geometry")
    if not isinstance(geometries, list) or len(geometries) != 1:
        raise GeometryError("model.geo.json must hold exactly one geometry")
    geometry = geometries[0]
    if not isinstance(geometry, dict):
        raise GeometryError("minecraft:geometry[0] must be an object")
    for key in REJECTED_GEOMETRY_KEYS:
        if key in geometry:
            raise GeometryError(f"{key} is not supported by the cosmetic model subset")

    description = geometry.get("description")
    if not isinstance(description, dict):
        raise GeometryError("geometry needs a description object")
    identifier = description.get("identifier")
    if not isinstance(identifier, str) or not identifier.startswith("geometry."):
        raise GeometryError("description.identifier must start with 'geometry.'")
    width = description.get("texture_width")
    height = description.get("texture_height")
    if not isinstance(width, int) or isinstance(width, bool) or width <= 0:
        raise GeometryError("description.texture_width must be a positive integer")
    if not isinstance(height, int) or isinstance(height, bool) or height <= 0:
        raise GeometryError("description.texture_height must be a positive integer")
    if width > 128 or height > 128:
        raise GeometryError(f"texture {width}x{height} exceeds the 128x128 limit")

    raw_bones = geometry.get("bones")
    if not isinstance(raw_bones, list) or not raw_bones:
        raise GeometryError("geometry needs a non-empty bones array")

    bones: dict[str, Bone] = {}
    for index, raw in enumerate(raw_bones):
        if not isinstance(raw, dict):
            raise GeometryError(f"bones[{index}] must be an object")
        unknown = set(raw) - BONE_KEYS
        if unknown:
            raise GeometryError(f"bones[{index}]: unsupported bone key(s) {sorted(unknown)}")
        name = raw.get("name")
        if not isinstance(name, str) or not name:
            raise GeometryError(f"bones[{index}] needs a name")
        if name in bones:
            raise GeometryError(f"duplicate bone name '{name}'")
        if name in ATTACHMENT_PIVOTS_BEDROCK and "parent" in raw:
            raise GeometryError(f"attachment bone '{name}' cannot have a parent")
        parent = raw.get("parent")
        if parent is not None and not isinstance(parent, str):
            raise GeometryError(f"bone '{name}': parent must be a bone name")
        pivot = _triple(raw.get("pivot", [0, 0, 0]), f"bone '{name}'.pivot")
        rotation = _triple(raw.get("rotation", [0, 0, 0]), f"bone '{name}'.rotation")
        if "mirror" in raw:
            raise GeometryError(
                f"bone '{name}': bone-level mirror is not supported — set mirror on the cubes"
            )
        cubes_raw = raw.get("cubes", [])
        if not isinstance(cubes_raw, list):
            raise GeometryError(f"bone '{name}': cubes must be an array")
        cubes = [parse_cube(c, f"bone '{name}'.cubes[{i}]") for i, c in enumerate(cubes_raw)]
        bones[name] = Bone(name, parent, pivot, rotation, cubes)

    # ---- parents, cycles, attachment shape -------------------------------
    for bone in bones.values():
        if bone.parent is not None and bone.parent not in bones:
            raise GeometryError(f"bone '{bone.name}': unknown parent '{bone.parent}'")
    for bone in bones.values():
        chain: set[str] = set()
        cursor = bone
        while cursor is not None:
            if cursor.name in chain:
                raise GeometryError(f"bone '{bone.name}' is part of a parent cycle")
            chain.add(cursor.name)
            cursor = bones.get(cursor.parent) if cursor.parent else None

    roots = [bone for bone in bones.values() if bone.parent is None]
    if len(roots) != 1:
        raise GeometryError(
            "geometry needs exactly one root: the player part it attaches to, got "
            + ", ".join(sorted(b.name for b in roots))
        )
    attachment = roots[0]
    if attachment.name not in ATTACHMENT_PIVOTS_BEDROCK:
        raise GeometryError(
            f"root bone '{attachment.name}' must be a player part ("
            + ", ".join(sorted(ATTACHMENT_PIVOTS_BEDROCK))
            + ")"
        )
    expected = ATTACHMENT_PIVOTS_BEDROCK[attachment.name]
    if any(abs(a - b) > 1e-6 for a, b in zip(attachment.pivot, expected)):
        raise GeometryError(
            f"attachment bone '{attachment.name}' must sit at {list(expected)}, "
            f"found {list(attachment.pivot)}"
        )
    if attachment.cubes:
        raise GeometryError(f"attachment bone '{attachment.name}' must not carry cubes")
    if any(abs(angle) > 1e-6 for angle in attachment.rotation):
        raise GeometryError(f"attachment bone '{attachment.name}' must not be rotated")

    for bone in bones.values():
        if bone.parent is not None:
            bones[bone.parent].children.append(bone.name)
        elif bone.name != attachment.name:  # pragma: no cover - guarded above
            raise GeometryError(f"bone '{bone.name}' is not reachable from the attachment")

    return Geometry(identifier, width, height, bones)


def check_cube_style(geometry: Geometry, item_id: str) -> list[str]:
    """Checks 2's texture/box rules: whole pixels, UV inside the sheet, budget."""

    errors: list[str] = []
    if geometry.cube_count() > MAX_CUBES:
        errors.append(f"{geometry.cube_count()} cubes exceeds the {MAX_CUBES} cube budget")
    legacy = item_id in LEGACY_FRACTIONAL_SIZES
    for bone in geometry.bones.values():
        for cube in bone.cubes:
            if not legacy and any(abs(v - round(v)) > 1e-9 for v in cube.size):
                errors.append(
                    f"bone '{bone.name}': cube size {list(cube.size)} is not whole pixels "
                    "(whole sizes keep every face on whole texels)"
                )
            span_u, span_v = cube.uv_span()
            u, v = cube.uv
            if u + span_u > geometry.texture_width + 1e-9 or v + span_v > geometry.texture_height + 1e-9:
                errors.append(
                    f"bone '{bone.name}': cube uv {[u, v]} unwraps to "
                    f"{_num(span_u)}x{_num(span_v)} px, outside the "
                    f"{geometry.texture_width}x{geometry.texture_height} texture"
                )
            if u < 0 or v < 0:
                errors.append(f"bone '{bone.name}': cube uv {[u, v]} is negative")
    return errors


def _num(value: float) -> str:
    return f"{value:g}"


# ---------------------------------------------------------------------------
# Pose sweep
# ---------------------------------------------------------------------------


def bone_transforms(geometry: Geometry, pose: dict[str, tuple[float, float, float]]):
    """Affine transform of every bone into the attachment part's frame.

    Returns ``{bone name: (rotation, origin)}`` where a point in the bone's own
    (pre-rotation) frame maps to ``origin + rotation @ point``. The attachment
    part contributes the identity because every check happens with the part at
    its rest pose; a bone directly under it therefore starts at its own pivot.
    """

    part = geometry.attachment
    part_pivot = bedrock_to_java_point(ATTACHMENT_PIVOTS_BEDROCK[part])
    out: dict[str, tuple[tuple[tuple[float, ...], ...], tuple[float, float, float]]] = {}

    for bone in geometry.ordered_bones():
        if bone.name == part:
            continue
        pivot = bedrock_to_java_point(bone.pivot)
        if bone.parent is None or bone.parent == part:
            parent_rotation, parent_origin = IDENTITY, part_pivot
        else:
            parent_rotation, parent_origin = out[bone.parent]
        delta = pose.get(bone.name, (0.0, 0.0, 0.0))
        local = rotation_matrix(
            bone.rotation[0] + delta[0],
            bone.rotation[1] + delta[1],
            bone.rotation[2] + delta[2],
        )
        offset = (
            pivot[0] - parent_origin[0],
            pivot[1] - parent_origin[1],
            pivot[2] - parent_origin[2],
        )
        moved = mat_vec(parent_rotation, offset)
        origin = (
            parent_origin[0] + moved[0],
            parent_origin[1] + moved[1],
            parent_origin[2] + moved[2],
        )
        out[bone.name] = (mat_mul(parent_rotation, local), origin)
    return out


def cube_corners(bone: Bone, cube: Cube) -> list[tuple[float, float, float]]:
    """The eight (inflated) box corners in the bone's own frame."""

    java_min = bedrock_to_java_cube(cube.origin, cube.size)
    bone_pivot = bedrock_to_java_point(bone.pivot)
    low = (
        java_min[0] - bone_pivot[0] - cube.inflate,
        java_min[1] - bone_pivot[1] - cube.inflate,
        java_min[2] - bone_pivot[2] - cube.inflate,
    )
    high = (
        java_min[0] - bone_pivot[0] + cube.size[0] + cube.inflate,
        java_min[1] - bone_pivot[1] + cube.size[1] + cube.inflate,
        java_min[2] - bone_pivot[2] + cube.size[2] + cube.inflate,
    )
    return [
        (x, y, z)
        for x in (low[0], high[0])
        for y in (low[1], high[1])
        for z in (low[2], high[2])
    ]


def footprint(
    geometry: Geometry, pose: dict[str, tuple[float, float, float]], clearance: float
) -> tuple[float, float, float, list[str]]:
    """Bounds of every cube for ``pose``, with the violations it breaks.

    Returns ``(min_y, max_y, max_abs_x, violations)`` in vanilla model space
    (part frame at rest, +Y down, origin on the neck line).
    """

    transforms = bone_transforms(geometry, pose)
    min_y = math.inf
    max_y = -math.inf
    max_x = 0.0
    violations: list[str] = []
    top_limit = HEAD_TOP_Y - clearance
    for bone in geometry.ordered_bones():
        if not bone.cubes:  # the attachment itself carries none
            continue
        rotation, origin = transforms[bone.name]
        for cube in bone.cubes:
            for corner in cube_corners(bone, cube):
                rotated = mat_vec(rotation, corner)
                x = origin[0] + rotated[0]
                y = origin[1] + rotated[1]
                z = origin[2] + rotated[2]
                min_y = min(min_y, y)
                max_y = max(max_y, y)
                max_x = max(max_x, abs(x))
                if y < top_limit - 1e-9:
                    violations.append(
                        f"bone '{bone.name}' reaches y={y:.2f}, {HEAD_TOP_Y - y:.2f} px above "
                        f"the head top (limit {_num(clearance)} px)"
                    )
                if y > KNEE_Y + 1e-9:
                    violations.append(
                        f"bone '{bone.name}' reaches y={y:.2f}, below the knees (y={_num(KNEE_Y)})"
                    )
                if abs(x) > MAX_ABS_X + 1e-9:
                    violations.append(
                        f"bone '{bone.name}' reaches |x|={abs(x):.2f}, past the "
                        f"{_num(MAX_ABS_X)} px width cap"
                    )
                del z
    return min_y, max_y, max_x, violations


# ---------------------------------------------------------------------------
# Compilation
# ---------------------------------------------------------------------------


def to_compiled(geometry: Geometry, item: dict) -> dict:
    """Build the runtime model document for one item."""

    part = geometry.attachment
    part_pivot = bedrock_to_java_point(ATTACHMENT_PIVOTS_BEDROCK[part])
    motion = item.get("motion") or {}
    bones = []
    for bone in geometry.ordered_bones():
        if bone.name == part:
            continue
        pivot = bedrock_to_java_point(bone.pivot)
        if bone.parent is None or bone.parent == part:
            parent_pivot = part_pivot
        else:
            parent_pivot = bedrock_to_java_point(geometry.bones[bone.parent].pivot)
        cubes = []
        for cube in bone.cubes:
            java_min = bedrock_to_java_cube(cube.origin, cube.size)
            cubes.append(
                {
                    "from": [
                        _round(java_min[0] - pivot[0]),
                        _round(java_min[1] - pivot[1]),
                        _round(java_min[2] - pivot[2]),
                    ],
                    "size": [_round(v) for v in cube.size],
                    "uv": [_round(v) for v in cube.uv],
                    "inflate": _round(cube.inflate),
                    "mirror": bool(cube.mirror),
                }
            )
        bones.append(
            {
                "name": bone.name,
                "pivot": [
                    _round(pivot[0] - parent_pivot[0]),
                    _round(pivot[1] - parent_pivot[1]),
                    _round(pivot[2] - parent_pivot[2]),
                ],
                "rotation": [_round(math.radians(a)) for a in bone.rotation],
                "cubes": cubes,
                "children": list(bone.children),
                "motion": [dict(driver) for driver in motion.get(bone.name, [])],
            }
        )
    return {
        "schema": 1,
        "id": item["id"],
        "render": item["render"],
        "hide_with": item["hide_with"],
        "texture": {"width": geometry.texture_width, "height": geometry.texture_height},
        "parts": [{"part": part, "bones": bones}],
    }


def _round(value: float) -> float:
    rounded = round(value, 6)
    if rounded == int(rounded):
        return float(int(rounded))
    return rounded
