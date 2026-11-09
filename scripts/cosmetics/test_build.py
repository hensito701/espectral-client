"""Tests for the cosmetics build (``scripts/cosmetics``).

Run with ``python -m pytest scripts/cosmetics -q``.
"""

from __future__ import annotations

import json
import math
import shutil
import sys
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent
if str(HERE) not in sys.path:
    sys.path.insert(0, str(HERE))

import catalog as catalog_mod  # noqa: E402
import geometry as geo  # noqa: E402
import pngio  # noqa: E402
from item import ItemError, driver_extremes, load_item, validate_item  # noqa: E402

#: A minimal valid item every test can start from.
BASE_ITEM = {
    "id": "demo",
    "kind": "model",
    "slot": "cabeza",
    "requires": "mvp",
    "order": 10,
    "name": {"es": "Demo", "en": "Demo"},
    "render": "cutout",
    "hide_with": "hat",
}


def geometry_doc(bones, width=64, height=32, identifier="geometry.espectral.demo"):
    return {
        "format_version": "1.12.0",
        "minecraft:geometry": [
            {
                "description": {
                    "identifier": identifier,
                    "texture_width": width,
                    "texture_height": height,
                },
                "bones": bones,
            }
        ],
    }


def simple_geometry(cube_origin=(-4, 28, -4), size=(8, 2, 8), uv=(0, 0), bone="band"):
    """One head-attached box wrapping the skull (a legal head item)."""

    return geometry_doc(
        [
            {"name": "head", "pivot": [0, 24, 0]},
            {
                "name": bone,
                "parent": "head",
                "pivot": [0, 24, 0],
                "cubes": [{"origin": list(cube_origin), "size": list(size), "uv": list(uv)}],
            },
        ]
    )


# ---------------------------------------------------------------------------
# Coordinate conversion
# ---------------------------------------------------------------------------


def test_java_y_is_24_minus_bedrock_y():
    assert geo.bedrock_to_java_point((1.5, 30.0, -2.0)) == (1.5, -6.0, -2.0)
    # A cube's Bedrock `origin` is its min corner; in Java space +Y is down,
    # so the min corner is the Bedrock box's TOP face.
    assert geo.bedrock_to_java_cube((0, 29, 0), (8, 2, 8)) == (0, -7, 0)
    assert geo.bedrock_to_java_cube((-4, 28, -4), (8, 2, 8)) == (-4, -6, -4)


def test_x_and_z_are_unchanged_by_the_conversion():
    for point in [(3.2, 11.0, -4.5), (-7.0, 0.0, 9.0)]:
        java = geo.bedrock_to_java_point(point)
        assert java[0] == point[0]
        assert java[2] == point[2]


def test_rotation_matrix_is_z_then_y_then_x():
    # 90 deg about Z sends +X to +Y.
    m = geo.rotation_matrix(0.0, 0.0, 90.0)
    x, y, z = geo.mat_vec(m, (1.0, 0.0, 0.0))
    assert (round(x, 6), round(y, 6), round(z, 6)) == (0.0, 1.0, 0.0)
    # 90 deg about X sends +Y to +Z.
    m = geo.rotation_matrix(90.0, 0.0, 0.0)
    x, y, z = geo.mat_vec(m, (0.0, 1.0, 0.0))
    assert (round(x, 6), round(y, 6), round(z, 6)) == (0.0, 0.0, 1.0)
    # Vanilla applies Z then Y then X, which is Rz*Ry*Rx; the X-then-Y order
    # would give a different point for the same three angles.
    assert geo.rotation_matrix(10.0, 20.0, 30.0) == geo.mat_mul(
        geo.rotation_matrix(0.0, 0.0, 30.0),
        geo.mat_mul(geo.rotation_matrix(0.0, 20.0, 0.0), geo.rotation_matrix(10.0, 0.0, 0.0)),
    )


def test_attachment_frame_places_cubes_on_the_right_side():
    # A cube 4 px toward Bedrock -X and -Z must land on the wearer's right (-X)
    # and face (-Z) side: the calibration the contract asks for.
    doc = simple_geometry(cube_origin=(-8, 28, -4), size=(4, 2, 8))
    geometry = geo.parse_geometry(doc)
    min_y, max_y, max_x, violations = geo.footprint(geometry, {}, 0.6)
    assert not violations
    assert max_x == pytest.approx(8.0)
    corners = geo.cube_corners(geometry.bones["band"], geometry.bones["band"].cubes[0])
    xs = [c[0] for c in corners]
    assert max(xs) <= 0.0, "a -X cube must sit on the wearer's right"


# ---------------------------------------------------------------------------
# Geometry rules
# ---------------------------------------------------------------------------


def test_attachment_bone_must_sit_at_the_part_pivot():
    doc = geometry_doc(
        [
            {"name": "head", "pivot": [0, 20, 0]},
            {"name": "band", "parent": "head", "pivot": [0, 24, 0],
             "cubes": [{"origin": [-4, 28, -4], "size": [8, 2, 8], "uv": [0, 0]}]},
        ]
    )
    with pytest.raises(geo.GeometryError, match="must sit at"):
        geo.parse_geometry(doc)


def test_attachment_bone_must_be_a_player_part():
    doc = geometry_doc([{"name": "neck", "pivot": [0, 24, 0]}])
    with pytest.raises(geo.GeometryError, match="must be a player part"):
        geo.parse_geometry(doc)


def test_attachment_bone_cannot_carry_cubes_or_rotation():
    doc = geometry_doc(
        [{"name": "head", "pivot": [0, 24, 0],
          "cubes": [{"origin": [-1, 28, -1], "size": [2, 2, 2], "uv": [0, 0]}]}]
    )
    with pytest.raises(geo.GeometryError, match="must not carry cubes"):
        geo.parse_geometry(doc)
    doc = geometry_doc([{"name": "head", "pivot": [0, 24, 0], "rotation": [10, 0, 0]}])
    with pytest.raises(geo.GeometryError, match="must not be rotated"):
        geo.parse_geometry(doc)


def test_duplicate_and_unknown_parent_are_rejected():
    doc = geometry_doc(
        [{"name": "head", "pivot": [0, 24, 0]}, {"name": "head", "pivot": [0, 24, 0]}]
    )
    with pytest.raises(geo.GeometryError, match="duplicate bone name"):
        geo.parse_geometry(doc)
    doc = geometry_doc(
        [{"name": "head", "pivot": [0, 24, 0]},
         {"name": "band", "parent": "cap", "pivot": [0, 24, 0]}]
    )
    with pytest.raises(geo.GeometryError, match="unknown parent"):
        geo.parse_geometry(doc)


def test_parent_cycle_is_rejected():
    doc = geometry_doc(
        [
            {"name": "head", "pivot": [0, 24, 0]},
            {"name": "a", "parent": "b", "pivot": [0, 24, 0]},
            {"name": "b", "parent": "a", "pivot": [0, 24, 0]},
        ]
    )
    with pytest.raises(geo.GeometryError, match="unknown parent|cycle"):
        geo.parse_geometry(doc)


def test_per_face_uv_is_rejected():
    doc = simple_geometry()
    doc["minecraft:geometry"][0]["bones"][1]["cubes"][0]["uv"] = {"north": {"uv": [0, 0], "uv_size": [8, 2]}}
    with pytest.raises(geo.GeometryError, match="box UV only"):
        geo.parse_geometry(doc)


def test_cube_with_extra_keys_is_rejected():
    doc = simple_geometry()
    doc["minecraft:geometry"][0]["bones"][1]["cubes"][0]["rotation"] = [0, 45, 0]
    with pytest.raises(geo.GeometryError, match="unsupported cube key"):
        geo.parse_geometry(doc)


def test_fractional_cube_size_is_rejected_except_for_the_ears():
    doc = simple_geometry(size=(8.5, 2, 8))
    geometry = geo.parse_geometry(doc)
    errors = geo.check_cube_style(geometry, "demo")
    assert any("whole pixels" in error for error in errors)
    # The two ear items are grandfathered in: their committed art matches the
    # fractional unwraps.
    assert geo.check_cube_style(geometry, "orejas_vulpinas") == []


def test_uv_box_outside_the_texture_is_rejected():
    doc = simple_geometry(uv=(60, 0))  # unwraps 32 px wide -> 92 > 64
    geometry = geo.parse_geometry(doc)
    errors = geo.check_cube_style(geometry, "demo")
    assert any("outside the" in error for error in errors)


def test_cube_budget_is_enforced():
    cubes = [{"origin": [-1, 28, -1], "size": [1, 1, 1], "uv": [0, 0]} for _ in range(65)]
    doc = geometry_doc(
        [
            {"name": "head", "pivot": [0, 24, 0]},
            {"name": "band", "parent": "head", "pivot": [0, 24, 0], "cubes": cubes},
        ]
    )
    geometry = geo.parse_geometry(doc)
    assert any("cube budget" in error for error in geo.check_cube_style(geometry, "demo"))


def test_texture_larger_than_128_is_rejected():
    doc = geometry_doc([{"name": "head", "pivot": [0, 24, 0]}], width=256, height=32)
    with pytest.raises(geo.GeometryError, match="128x128"):
        geo.parse_geometry(doc)


def test_locators_are_rejected():
    doc = simple_geometry()
    doc["minecraft:geometry"][0]["locators"] = {}
    with pytest.raises(geo.GeometryError, match="locators"):
        geo.parse_geometry(doc)


# ---------------------------------------------------------------------------
# Footprint
# ---------------------------------------------------------------------------


def _depth(geometry: geo.Geometry, pose) -> float:
    """How far the geometry's cubes spread along Z (rest vs posed)."""

    transforms = geo.bone_transforms(geometry, pose)
    zs = []
    for bone in geometry.ordered_bones():
        if not bone.cubes:
            continue
        rotation, origin = transforms[bone.name]
        for cube in bone.cubes:
            for corner in geo.cube_corners(bone, cube):
                rotated = geo.mat_vec(rotation, corner)
                zs.append(origin[2] + rotated[2])
    return max(zs) - min(zs)


def test_legal_bandana_clears_the_head_by_less_than_the_default():
    # 0.55 px of inflate sits inside the 0.6 px hat-layer allowance.
    doc = simple_geometry()
    doc["minecraft:geometry"][0]["bones"][1]["cubes"][0]["inflate"] = 0.55
    geometry = geo.parse_geometry(doc)
    # Top of the band (y = -6) plus the inflate reaches -6.55, inside the
    # -8.6 limit a 0.6 px clearance allows.
    min_y, _max_y, _max_x, violations = geo.footprint(geometry, {}, 0.6)
    assert violations == []
    assert min_y == pytest.approx(-6.55)


def test_nothing_may_reach_above_the_head_top():
    doc = simple_geometry(cube_origin=(-4, 32, -4), size=(8, 2, 8))
    geometry = geo.parse_geometry(doc)
    _min_y, _max_y, _max_x, violations = geo.footprint(geometry, {}, 0.6)
    assert any("above the head top" in violation for violation in violations)


def test_ear_exception_allows_five_pixels_above_the_head_top():
    doc = simple_geometry(cube_origin=(-4, 32, -4), size=(8, 2, 8))
    geometry = geo.parse_geometry(doc)
    _min_y, _max_y, _max_x, violations = geo.footprint(geometry, {}, 5.0)
    assert violations == []


def test_nothing_may_reach_below_the_knees():
    doc = geometry_doc(
        [
            {"name": "body", "pivot": [0, 24, 0]},
            {"name": "tail", "parent": "body", "pivot": [0, 24, 0],
             "cubes": [{"origin": [-1, 4, 2], "size": [2, 4, 2], "uv": [0, 0]}]},
        ]
    )
    geometry = geo.parse_geometry(doc)
    _min_y, max_y, _max_x, violations = geo.footprint(geometry, {}, 0.6)
    assert max_y > geo.KNEE_Y
    assert any("below the knees" in violation for violation in violations)


def test_too_wide_is_rejected():
    doc = simple_geometry(cube_origin=(-13, 28, -4), size=(4, 2, 8))
    geometry = geo.parse_geometry(doc)
    _min_y, _max_y, max_x, violations = geo.footprint(geometry, {}, 0.6)
    assert max_x > geo.MAX_ABS_X
    assert any("width cap" in violation for violation in violations)


def test_a_rotation_only_breaks_the_cap_at_a_driver_extreme():
    # A long tail lies along +Z behind the head: legal at rest (z is unbounded),
    # but a driver that swings it down takes it past the knees.
    doc = geometry_doc(
        [
            {"name": "head", "pivot": [0, 24, 0]},
            {"name": "tail", "parent": "head", "pivot": [0, 28, 4],
             "cubes": [{"origin": [-1, 26, 4], "size": [2, 2, 24], "uv": [0, 0]}]},
        ]
    )
    geometry = geo.parse_geometry(doc)
    assert geo.footprint(geometry, {}, 0.6)[3] == []
    swung = geo.footprint(geometry, {"tail": (-90.0, 0.0, 0.0)}, 0.6)
    assert any("below the knees" in violation for violation in swung[3])


def test_bone_rotation_moves_the_whole_child_chain():
    # A two-segment tail hanging behind the head: rotating the upper segment
    # must carry the lower one, which the single-bone bound would miss.
    doc = geometry_doc(
        [
            {"name": "head", "pivot": [0, 24, 0]},
            {"name": "upper", "parent": "head", "pivot": [0, 28, 4],
             "cubes": [{"origin": [-1, 24, 4], "size": [2, 4, 2], "uv": [0, 0]}]},
            {"name": "lower", "parent": "upper", "pivot": [0, 24, 4],
             "cubes": [{"origin": [-1, 20, 4], "size": [2, 4, 2], "uv": [8, 0]}]},
        ]
    )
    geometry = geo.parse_geometry(doc)

    rest = geo.footprint(geometry, {}, 0.6)
    posed = geo.footprint(geometry, {"upper": (-70.0, 0.0, 0.0)}, 0.6)
    rest_depth = _depth(geometry, {})
    posed_depth = _depth(geometry, {"upper": (-70.0, 0.0, 0.0)})
    assert rest_depth == pytest.approx(2.0)  # the cube sits inside |z| 4..6
    assert posed_depth > rest_depth + 2.0, "the lower segment must swing out with its parent"
    assert rest[:3] != posed[:3]


# ---------------------------------------------------------------------------
# Texture rules
# ---------------------------------------------------------------------------


def test_alpha_rule_allows_zero_and_200_but_not_199():
    rows = [[(0, 0, 0, 0), (10, 20, 30, 200)], [(1, 2, 3, 199), (4, 5, 6, 255)]]
    assert pngio.alpha_rule_violations(2, 2, rows)


def test_texture_txt_round_trip():
    text = "size 2 2\npalette\n. transparent\nR #ff0000\nrows\nR.\n.R\n"
    width, height, rows = pngio.parse_texture_txt(text)
    assert (width, height) == (2, 2)
    assert rows[0][0] == (255, 0, 0, 255)
    assert rows[0][1][3] == 0
    assert rows[1][0][3] == 0


def test_texture_txt_accepts_alpha_colours():
    text = "size 1 1\npalette\nW #112233aa\nrows\nW\n"
    assert pngio.parse_texture_txt(text)[2][0][0] == (0x11, 0x22, 0x33, 0xAA)


@pytest.mark.parametrize(
    "text, message",
    [
        ("palette\n. transparent\nrows\n.\n", "'size <width> <height>'"),
        ("size 2 2\n. transparent\nrows\n..\n..\n", "missing 'palette"),
        ("size 2 2\npalette\n. transparent\nrows\n..\n", "expected 2 rows"),
        ("size 2 2\npalette\n. transparent\nrows\n...\n..\n", "3 characters"),
        ("size 2 2\npalette\n. transparent\nrows\n.X\n..\n", "undefined palette key"),
        ("size 2 2\npalette\n. transparent\nR red\nrows\nR.\n..\n", "must start with '#'"),
        ("size 2 2\npalette\n. transparent\nR #ff0000\nR #00ff00\nrows\nR.\n..\n", "declared twice"),
        ("size 2 2\npalette\nX transparent\nrows\n..\n..\n", "only '.' may be transparent"),
        ("size 2 2\npalette\nQ opaque\nrows\n..\n..\n", "must start with '#'"),

    ],
)
def test_texture_txt_errors(text, message):
    with pytest.raises(pngio.TextureError, match=message):
        pngio.parse_texture_txt(text)


def test_png_writer_is_deterministic(tmp_path):
    rows = [[(1, 2, 3, 255), (4, 5, 6, 0)]]
    first = pngio.png_bytes(2, 1, rows)
    second = pngio.png_bytes(2, 1, rows)
    assert first == second
    width, height, back = pngio.read_png(_write(tmp_path / "t.png", first))
    assert (width, height) == (2, 1)
    assert back == [[(1, 2, 3, 255), (4, 5, 6, 0)]]


def _write(path: Path, data: bytes) -> Path:
    path.write_bytes(data)
    return path


# ---------------------------------------------------------------------------
# item.json
# ---------------------------------------------------------------------------


def test_item_defaults_follow_the_slot():
    item = validate_item(dict(BASE_ITEM, slot="cabeza"), "demo")
    assert item["hide_with"] == "hat"
    assert item["head_clearance_px"] == pytest.approx(0.6)
    assert validate_item(dict(BASE_ITEM, slot="espalda", hide_with="cape"), "demo")["hide_with"] == "cape"


def test_item_id_must_match_the_folder():
    with pytest.raises(ItemError, match="must equal the folder name"):
        validate_item(dict(BASE_ITEM, id="other"), "demo")


def test_item_id_charset_is_checked():
    with pytest.raises(ItemError, match="must be"):
        validate_item(dict(BASE_ITEM, id="Demo!", slot="cabeza"), "Demo!")


def test_unknown_item_field_is_rejected():
    with pytest.raises(ItemError, match="unknown item field"):
        validate_item(dict(BASE_ITEM, colour="red"), "demo")


def test_unknown_slot_is_rejected():
    with pytest.raises(ItemError, match="is not in catalog slots"):
        validate_item(dict(BASE_ITEM, slot="brazos"), "demo", ["espalda", "cuello", "cabeza", "elitros"])


def test_names_need_both_languages():
    with pytest.raises(ItemError, match="exactly 'es' and 'en'"):
        validate_item(dict(BASE_ITEM, name={"es": "Solo"}), "demo")


def test_clearance_is_capped_per_item():
    with pytest.raises(ItemError, match="exceeds 0.6"):
        validate_item(dict(BASE_ITEM, head_clearance_px=3), "demo")
    assert validate_item(dict(BASE_ITEM, id="orejas_felinas"), "orejas_felinas", None)["head_clearance_px"] == 0.6
    ears = validate_item(
        dict(BASE_ITEM, id="orejas_felinas", head_clearance_px=5.0), "orejas_felinas"
    )
    assert ears["head_clearance_px"] == 5.0
    with pytest.raises(ItemError, match="exceeds 5"):
        validate_item(dict(BASE_ITEM, id="orejas_felinas", head_clearance_px=6.0), "orejas_felinas")


def test_elytra_items_need_frames_and_no_motion():
    ok = validate_item(dict(BASE_ITEM, kind="elytra", slot="elitros", frames=4), "demo")
    assert ok["frames"] == 4
    with pytest.raises(ItemError, match="needs 'frames'"):
        validate_item(dict(BASE_ITEM, kind="elytra", slot="elitros"), "demo")
    with pytest.raises(ItemError, match="no geometry, so no motion"):
        validate_item(
            dict(
                BASE_ITEM,
                kind="elytra",
                slot="elitros",
                frames=1,
                motion={"tail": [{"type": "land", "impulse": 5}]},
            ),
            "demo",
        )
    with pytest.raises(ItemError, match="only elytra items carry 'frames'"):
        validate_item(dict(BASE_ITEM, frames=2), "demo")


def test_swing_bounds_are_checked():
    base = {"type": "swing", "axis": "x", "rest": 10, "min": -5, "max": 70}
    item = validate_item(dict(BASE_ITEM, motion={"tail": [base]}), "demo")
    assert item["motion"]["tail"][0]["rest"] == 10
    with pytest.raises(ItemError, match="min <= rest <= max"):
        validate_item(dict(BASE_ITEM, motion={"tail": [dict(base, rest=90)]}), "demo")
    with pytest.raises(ItemError, match="stiffness"):
        validate_item(dict(BASE_ITEM, motion={"tail": [dict(base, stiffness=0.9)]}), "demo")
    with pytest.raises(ItemError, match="damping"):
        validate_item(dict(BASE_ITEM, motion={"tail": [dict(base, damping=0.2)]}), "demo")
    with pytest.raises(ItemError, match="axis must be one of"):
        validate_item(dict(BASE_ITEM, motion={"tail": [dict(base, axis="y")]}), "demo")


def test_driver_field_typos_are_rejected():
    with pytest.raises(ItemError, match="unknown driver type"):
        validate_item(dict(BASE_ITEM, motion={"tail": [{"type": "wobble"}]}), "demo")
    with pytest.raises(ItemError, match="unsupported field"):
        validate_item(
            dict(BASE_ITEM, motion={"tail": [{"type": "head_turn", "gain": 1, "gaim": 2}]}), "demo"
        )


def test_react_bounds_and_states_are_checked():
    with pytest.raises(ItemError, match="needs at least one of"):
        validate_item(dict(BASE_ITEM, motion={"tail": [{"type": "react", "rate": 0.2}]}), "demo")
    with pytest.raises(ItemError, match="rate 0.01 outside"):
        validate_item(
            dict(BASE_ITEM, motion={"tail": [{"type": "react", "rate": 0.01, "sneak": {"x": 1}}]}),
            "demo",
        )
    with pytest.raises(ItemError, match="unsupported key"):
        validate_item(
            dict(BASE_ITEM, motion={"tail": [{"type": "react", "sneak": {"w": 1}}]}), "demo"
        )


def test_driver_extremes_cover_every_reachable_angle():
    drivers = [
        {"type": "swing", "axis": "x", "rest": 0, "min": -10, "max": 20},
        {"type": "react", "rate": 0.2, "sneak": {"x": 5, "z": -3}},
        {"type": "head_turn", "axis": "x", "gain": 1, "max": 15, "decay": 0.7},
        {"type": "land", "axis": "x", "impulse": 12, "decay": 0.8},
        {"type": "flap", "axis": "y", "closed": 60, "open": -10, "beat": 12},
    ]
    angles = [delta for _label, delta in driver_extremes(drivers)]
    assert (0.0, 0.0, 0.0) not in angles
    assert (-10.0, 0.0, 0.0) in angles and (20.0, 0.0, 0.0) in angles
    assert (5.0, 0.0, -3.0) in angles
    assert (15.0, 0.0, 0.0) in angles and (-15.0, 0.0, 0.0) in angles
    assert (12.0, 0.0, 0.0) in angles
    # closed 60, open -10 with a +/-12 beat: -22 .. 2
    assert (0.0, 60.0, 0.0) in angles
    assert (0.0, 2.0, 0.0) in angles and (0.0, -22.0, 0.0) in angles


# ---------------------------------------------------------------------------
# Catalog
# ---------------------------------------------------------------------------


def test_catalog_shape_matches_the_shipped_file():
    root = HERE.parent.parent
    base = catalog_mod.load_base(root / "branding-mod/cosmetics/catalog.base.json")
    assert base["schema"] == 1
    assert base["slots"] == ["espalda", "cuello", "cabeza", "elitros"]
    assert base["exclusive"] == []
    assert [emote["id"] for emote in base["emotes"]] == [
        "saludo",
        "reverencia",
        "aplauso",
        "sentarse",
        "invocacion",
    ]


def test_catalog_orders_by_slot_then_order_then_id():
    base = {
        "schema": 1,
        "slots": ["espalda", "cuello", "cabeza", "elitros"],
        "exclusive": [],
        "emotes": [],
    }
    items = [
        {"id": "c2", "slot": "cabeza", "requires": "mvp", "order": 20, "name": {"es": "C2", "en": "C2"}},
        {"id": "c1", "slot": "cabeza", "requires": "mvp", "order": 10, "name": {"es": "C1", "en": "C1"}},
        {"id": "a1", "slot": "espalda", "requires": "mvp", "order": 99, "name": {"es": "A1", "en": "A1"}},
        {"id": "c0", "slot": "cabeza", "requires": "mvp", "order": 10, "name": {"es": "C0", "en": "C0"}},
    ]
    text = catalog_mod.render(base, items)
    document = json.loads(text)
    assert [item["id"] for item in document["items"]] == ["a1", "c0", "c1", "c2"]
    assert document["items"][0] == {
        "id": "a1",
        "slot": "espalda",
        "requires": "mvp",
        "name": {"es": "A1", "en": "A1"},
    }


def test_catalog_one_item_per_line_and_single_trailing_newline():
    root = HERE.parent.parent
    text = (root / "native/engine/data/cosmetics-catalog.json").read_text(encoding="utf-8")
    assert text.endswith("}\n") and not text.endswith("\n\n")
    assert "\r" not in text
    assert '"items": [\n' in text
    in_items = False
    for line in text.splitlines():
        stripped = line.strip()
        if stripped == '"items": [':
            in_items = True
        elif stripped.startswith('"emotes"'):
            in_items = False
        elif in_items and stripped.startswith("{"):
            assert line.count("{") == line.count("}")


def test_catalog_rejects_a_broken_exclusive_rule(tmp_path):
    base = {"schema": 1, "slots": ["cabeza"], "exclusive": [["cuello", ["a"]]], "emotes": []}
    path = tmp_path / "catalog.base.json"
    path.write_text(json.dumps(base), encoding="utf-8")
    with pytest.raises(catalog_mod.CatalogError, match="unknown slot"):
        catalog_mod.load_base(path)


def test_catalog_detects_exclusive_rules_naming_unknown_items():
    base = {"schema": 1, "slots": ["cuello"], "exclusive": [["cuello", ["gone"]]], "emotes": []}
    problems = catalog_mod.check_against_items(base, [])
    assert any("unknown item" in problem for problem in problems)


# ---------------------------------------------------------------------------
# End to end
# ---------------------------------------------------------------------------


@pytest.fixture()
def build_root(tmp_path, monkeypatch):
    """A private copy of the real sources + outputs the build can write into."""

    import build as build_mod

    root = tmp_path / "repo"
    shutil.copytree(HERE.parent.parent / "branding-mod/cosmetics", root / "branding-mod/cosmetics")
    (root / "scripts").mkdir(parents=True)
    monkeypatch.setattr(build_mod, "ROOT", root)
    monkeypatch.setattr(build_mod, "SOURCE_DIR", root / "branding-mod/cosmetics")
    monkeypatch.setattr(build_mod, "BASE_FILE", root / "branding-mod/cosmetics/catalog.base.json")
    monkeypatch.setattr(build_mod, "MODEL_DIR", root / "assets/cosmetics/models")
    monkeypatch.setattr(build_mod, "TEXTURE_DIR", root / "assets/textures/cosmetics")
    monkeypatch.setattr(build_mod, "CATALOG_FILE", root / "catalog.json")
    monkeypatch.setattr(build_mod, "SERVICE_CATALOG", root / "service-catalog.json")
    monkeypatch.setattr(build_mod, "PREVIEW_DIR", root / "preview")
    monkeypatch.setattr(build_mod, "SYNC_SCRIPT", root / "missing-sync.mjs")
    monkeypatch.setattr(build_mod, "run_sync", lambda: [])
    return build_mod, root


def test_build_is_deterministic(build_root):
    build_mod, root = build_root
    assert build_mod.main([]) == 0
    first = {
        path: path.read_bytes()
        for path in sorted(root.rglob("*"))
        if path.is_file() and path.suffix in {".json", ".png"}
    }
    assert build_mod.main([]) == 0
    second = {
        path: path.read_bytes()
        for path in sorted(root.rglob("*"))
        if path.is_file() and path.suffix in {".json", ".png"}
    }
    assert first.keys() == second.keys()
    for path, data in first.items():
        assert data == second[path], path


def test_build_prunes_retired_textures(build_root):
    build_mod, root = build_root
    stale = root / "assets/textures/cosmetics/manto_espectral.png"
    stale.parent.mkdir(parents=True, exist_ok=True)
    stale.write_bytes(b"not a png")
    assert build_mod.main([]) == 0
    assert not stale.exists()


def test_check_reports_missing_outputs(build_root):
    build_mod, _root = build_root
    assert build_mod.main(["--check"]) == 1


def test_check_passes_after_a_build(build_root):
    build_mod, _root = build_root
    assert build_mod.main([]) == 0
    assert build_mod.main(["--check"]) == 0


def test_check_flags_a_hand_edited_output(build_root):
    build_mod, root = build_root
    assert build_mod.main([]) == 0
    model = root / "assets/cosmetics/models/orejas_vulpinas.json"
    model.write_bytes(model.read_bytes() + b"\n")
    assert build_mod.main(["--check"]) == 1
