#!/usr/bin/env python3
"""Cosmetics build: validate every item, compile it for the mod, regenerate the
catalog and (with ``--preview``) render a review sheet.

Contract: ``docs/cosmetics-pipeline.md``. Usage::

    python scripts/cosmetics/build.py                 # build everything
    python scripts/cosmetics/build.py --check         # validate only, write nothing
    python scripts/cosmetics/build.py --item <id>     # one item
    python scripts/cosmetics/build.py --preview       # build + preview sheets

The build is deterministic: unchanged sources produce byte-identical outputs,
so the generated files can be committed and diffed.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
if str(HERE) not in sys.path:
    sys.path.insert(0, str(HERE))

import catalog as catalog_mod  # noqa: E402
import geometry as geo  # noqa: E402
import pngio  # noqa: E402
from item import ItemError, driver_extremes, load_item, neutral_pose  # noqa: E402

ROOT = HERE.parent.parent
SOURCE_DIR = ROOT / "branding-mod" / "cosmetics"
BASE_FILE = SOURCE_DIR / "catalog.base.json"
MENU_ASSETS = ROOT / "branding-mod" / "src" / "main" / "resources" / "assets" / "espectral-menu"
MODEL_DIR = MENU_ASSETS / "cosmetics" / "models"
TEXTURE_DIR = MENU_ASSETS / "textures" / "cosmetics"
CATALOG_FILE = ROOT / "native" / "engine" / "data" / "cosmetics-catalog.json"
SERVICE_CATALOG = ROOT / "services" / "espectral-id" / "app" / "cosmetics-catalog.json"
PREVIEW_DIR = ROOT / "branding-mod" / "build" / "cosmetics-preview"
SYNC_SCRIPT = ROOT / "scripts" / "sync-suite-registry.mjs"

AXES = "xyz"


class BuildError(Exception):
    """Fatal build problem (not tied to one item): the run stops."""


class ItemResult:
    """Everything one item produces, held in memory until it is written."""

    def __init__(self, item: dict):
        self.item = item
        self.outputs: dict[Path, bytes] = {}
        self.errors: list[str] = []
        self.preview_rows: list[tuple[str, dict]] = []
        self.geometry: geo.Geometry | None = None
        self.texture: list[list[pngio.RGBA]] | None = None

    @property
    def valid(self) -> bool:
        return not self.errors


# ---------------------------------------------------------------------------
# Discovery
# ---------------------------------------------------------------------------


def discover() -> tuple[dict, list[Path]]:
    """``(catalog base, item folders in id order)``."""

    if not BASE_FILE.is_file():
        raise BuildError(f"missing {BASE_FILE.relative_to(ROOT)}")
    base = catalog_mod.load_base(BASE_FILE)
    folders = sorted(p for p in SOURCE_DIR.iterdir() if p.is_dir() and (p / "item.json").is_file())
    if not folders:
        raise BuildError(f"no item folders under {SOURCE_DIR.relative_to(ROOT)}")
    return base, folders


# ---------------------------------------------------------------------------
# Compilation
# ---------------------------------------------------------------------------


def load_texture(folder: Path, item: dict, result: ItemResult, geometry: geo.Geometry | None,
                check: bool):
    """Read ``texture.txt`` / ``texture.png`` into ``(width, height, pixels)``.

    ``texture.txt`` is the authoring source when present, and a build writes the
    PNG back from it. Under ``--check`` the committed PNG must already match,
    which is what keeps the two sources from drifting apart.
    """

    item_id = item["id"]
    txt = folder / "texture.txt"
    png = folder / "texture.png"
    expected = (geometry.texture_width, geometry.texture_height) if geometry else None

    if txt.is_file():
        width, height, rows = pngio.parse_texture_txt(txt.read_text(encoding="utf-8"))
        if expected and (width, height) != expected:
            result.errors.append(
                f"{item_id}: texture.txt is {width}x{height} but the geometry expects "
                f"{expected[0]}x{expected[1]}"
            )
        if check and png.is_file():
            have_w, have_h, have_rows = pngio.read_png(png)
            if (have_w, have_h) != (width, height):
                result.errors.append(
                    f"{item_id}: texture.png is {have_w}x{have_h} but texture.txt is "
                    f"{width}x{height} — run the build to regenerate it"
                )
            elif have_rows != rows:
                result.errors.append(
                    f"{item_id}: texture.png does not match texture.txt — run the build "
                    "to regenerate it"
                )
        return width, height, rows
    if png.is_file():
        width, height, rows = pngio.read_png(png)
        if expected and (width, height) != expected:
            result.errors.append(
                f"{item_id}: texture.png is {width}x{height} but the geometry expects "
                f"{expected[0]}x{expected[1]}"
            )
        return width, height, rows
    result.errors.append(f"{item_id}: no texture.txt or texture.png")
    return None


def compile_model_item(folder: Path, item: dict, result: ItemResult, check: bool) -> None:
    geo_file = folder / "model.geo.json"
    if not geo_file.is_file():
        result.errors.append(f"{item['id']}: model items need model.geo.json")
        return
    try:
        raw = json.loads(geo_file.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        result.errors.append(f"{item['id']}: model.geo.json: invalid JSON ({exc})")
        return
    try:
        geometry = geo.parse_geometry(raw)
    except geo.GeometryError as exc:
        result.errors.append(f"{item['id']}: {exc}")
        return
    result.geometry = geometry
    result.errors.extend(geo.check_cube_style(geometry, item["id"]))
    result.errors.extend(motion_bone_errors(geometry, item))

    texture = load_texture(folder, item, result, geometry, check)
    if texture is None:
        return
    width, height, rows = texture
    result.texture = rows
    result.errors.extend(
        f"{item['id']}: {problem}" for problem in pngio.alpha_rule_violations(width, height, rows)
    )

    result.errors.extend(footprint_errors(geometry, item))
    if result.errors:
        return

    png = pngio.png_bytes(width, height, rows)
    result.outputs[MODEL_DIR / f"{item['id']}.json"] = (
        json.dumps(geo.to_compiled(geometry, item), indent=2, ensure_ascii=False) + "\n"
    ).encode("utf-8")
    result.outputs[TEXTURE_DIR / f"{item['id']}.png"] = png
    if (folder / "texture.txt").is_file():
        # texture.txt -> texture.png: keep the authoring folder's PNG in sync
        # so Blockbench (which only opens PNGs) never sees stale art.
        result.outputs[folder / "texture.png"] = png
    result.preview_rows = preview_rows(item)


def motion_bone_errors(geometry: geo.Geometry, item: dict) -> list[str]:
    """A driver on a bone the geometry does not have (or that draws nothing) is dead."""

    errors: list[str] = []
    for bone_name in item["motion"]:
        bone = geometry.bones.get(bone_name)
        if bone is None:
            errors.append(f"{item['id']}: motion names unknown bone '{bone_name}'")
        elif bone_name == geometry.attachment:
            errors.append(
                f"{item['id']}: motion on attachment bone '{bone_name}' would move the player"
            )
        elif not bone.cubes and not bone.children:
            errors.append(f"{item['id']}: motion on empty bone '{bone_name}'")
    return errors


def footprint_errors(geometry: geo.Geometry, item: dict) -> list[str]:
    """Check 4: the footprint at rest and at every reachable driver extreme."""

    clearance = item["head_clearance_px"]
    errors: list[str] = []

    def check(pose, label: str) -> None:
        _min_y, _max_y, _max_x, violations = geo.footprint(geometry, pose, clearance)
        errors.extend(f"{item['id']}: {label}: {violation}" for violation in violations)

    # Driver angles are contributions the runtime adds to the bone's authored
    # rotation, and each rests at its own neutral (a swing at `rest`, a flap at
    # `closed`), so both poses are checked: authored rest and driver neutral.
    check({}, "authored rest pose")
    check(neutral_pose(item), "driver neutral pose")
    for label, pose in single_bone_extremes(item):
        check(pose, label)
    for label, pose in combined_extremes(item):
        check(pose, label)
    return errors


def single_bone_extremes(item: dict):
    """One pose per reachable extreme of one bone's drivers, plus its all-min/all-max."""

    poses: list[tuple[str, dict]] = []
    for bone, drivers in item["motion"].items():
        extremes = driver_extremes(drivers)
        for label, delta in extremes:
            poses.append((f"{bone} {label}", {bone: delta}))
        lows = tuple(min(delta[i] for _label, delta in extremes) for i in range(3))
        highs = tuple(max(delta[i] for _label, delta in extremes) for i in range(3))
        poses.append((f"{bone} all minimum", {bone: lows}))
        poses.append((f"{bone} all maximum", {bone: highs}))
    return poses


def combined_extremes(item: dict):
    """Cross-bone combinations the per-bone sweep cannot reach.

    Every bone sits at its lowest (and then its highest) angle on one axis at a
    time: two wings opening at once is the widest the item ever gets, and two
    ear bones flattening together is the tallest, while neither shows up in a
    single-bone pose.
    """

    per_bone = {
        bone: driver_extremes(drivers)
        for bone, drivers in item["motion"].items()
        if drivers
    }
    if len(per_bone) < 2:
        return []
    poses: list[tuple[str, dict]] = []
    for axis in range(3):
        lows = {
            bone: tuple(min(delta[i] for _label, delta in extremes) for i in range(3))
            for bone, extremes in per_bone.items()
        }
        highs = {
            bone: tuple(max(delta[i] for _label, delta in extremes) for i in range(3))
            for bone, extremes in per_bone.items()
        }
        # Filling every axis here is deliberate: min on one axis and max on the
        # others has no physical meaning, so both variants stay uniform.
        poses.append((f"every bone at its {AXES[axis]} minimum", lows))
        poses.append((f"every bone at its {AXES[axis]} maximum", highs))
    return poses


def compile_elytra_item(folder: Path, item: dict, result: ItemResult) -> None:
    """Elytra skins: copy the frames to the file names the mod asks for."""

    item_id = item["id"]
    count = item["frames"]
    frames_dir = folder / "frames"
    expected = {f"elytra_{index}.png" for index in range(count)}
    if not frames_dir.is_dir():
        result.errors.append(f"{item_id}: an elytra item needs frames/")
        return
    present = {path.name for path in frames_dir.glob("*.png")}
    for missing in sorted(expected - present):
        result.errors.append(f"{item_id}: missing frames/{missing}")
    for extra in sorted(present - expected):
        result.errors.append(f"{item_id}: unexpected frame frames/{extra}")
    if result.errors:
        return

    sizes = set()
    for index in range(count):
        width, height, rows = pngio.read_png(frames_dir / f"elytra_{index}.png")
        sizes.add((width, height))
        result.errors.extend(
            f"{item_id}: frames/elytra_{index}.png: {problem}"
            for problem in pngio.alpha_rule_violations(width, height, rows)
        )
        if width > 128 or height > 128:
            result.errors.append(f"{item_id}: frames/elytra_{index}.png exceeds 128x128")
        name = f"{item_id}.png" if count == 1 else f"{item_id}_{index}.png"
        result.outputs[TEXTURE_DIR / name] = pngio.png_bytes(width, height, rows)
    if len(sizes) > 1:
        result.errors.append(f"{item_id}: frames differ in size: {sorted(sizes)}")


# ---------------------------------------------------------------------------
# Preview rows
# ---------------------------------------------------------------------------


def preview_rows(item: dict):
    """``[(label, pose), …]``: the rest pose plus every state that bends the item.

    A row's pose is the full driver contribution the runtime writes into the
    bones, so the sheet shows exactly what the game would draw for that state.
    """

    rows: list[tuple[str, dict]] = [("rest", pose_for(item))]
    for state in ("sneak", "sprint", "airborne", "glide"):
        pose = pose_for(item, react=state)
        if pose != pose_for(item):
            rows.append((state, pose))

    lowest = pose_for(item, swing="min")
    highest = pose_for(item, swing="max")
    if lowest != pose_for(item) or highest != pose_for(item):
        rows.append(("swing min", lowest))
        rows.append(("swing max", highest))

    if any(d["type"] == "flap" for drivers in item["motion"].values() for d in drivers):
        rows.append(("flap open", pose_for(item, flap=True)))
    if any(d["type"] == "land" for drivers in item["motion"].values() for d in drivers):
        rows.append(("landing", pose_for(item, land=True)))
    if any(d["type"] == "head_turn" for drivers in item["motion"].values() for d in drivers):
        rows.append(("head turn", pose_for(item, head_turn=True)))
    return rows


def pose_for(item: dict, react: str | None = None, swing: str | None = None,
             flap: bool = False, land: bool = False, head_turn: bool = False):
    """Full driver contribution of every bone for one preview state."""

    pose: dict[str, tuple[float, float, float]] = {}
    for bone, drivers in item["motion"].items():
        x = y = z = 0.0
        for driver in drivers:
            kind = driver["type"]
            axis = driver.get("axis", "x")
            if kind == "react":
                offsets = driver.get(react) if react else None
                if offsets:
                    x += float(offsets.get("x", 0.0))
                    y += float(offsets.get("y", 0.0))
                    z += float(offsets.get("z", 0.0))
                continue
            if kind == "swing":
                angle = float(driver["rest"]) if swing is None else float(driver[swing])
            elif kind == "flap":
                angle = (
                    float(driver["open"]) + float(driver.get("beat", 0.0))
                    if flap
                    else float(driver["closed"])
                )
            elif kind == "land":
                angle = float(driver["impulse"]) if land else 0.0
            elif kind == "head_turn":
                angle = float(driver.get("max", 30.0)) if head_turn else 0.0
            else:
                continue
            if axis == "y":
                y += angle
            elif axis == "z":
                z += angle
            else:
                x += angle
        if x or y or z:
            pose[bone] = (x, y, z)
    return pose


# ---------------------------------------------------------------------------
# Writing
# ---------------------------------------------------------------------------


def write_if_changed(path: Path, data: bytes, written: list, unchanged: list) -> None:
    if path.is_file() and path.read_bytes() == data:
        unchanged.append(path)
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    written.append(path)


def prune(directory: Path, expected: set[Path], written: list) -> list[Path]:
    """Delete generated files no item produces any more (retired cosmetics)."""

    removed: list[Path] = []
    if not directory.is_dir():
        return removed
    for path in sorted(directory.iterdir()):
        if not path.is_file() or path in expected:
            continue
        path.unlink()
        removed.append(path)
    return removed


def run_sync() -> list[str]:
    """Copy the catalog into the mod through the suite registry script."""

    try:
        completed = subprocess.run(
            ["node", str(SYNC_SCRIPT)],
            cwd=str(ROOT),
            capture_output=True,
            text=True,
            check=False,
        )
    except OSError as exc:
        return [f"could not run {SYNC_SCRIPT.name}: {exc}"]
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout).strip()
        return [f"{SYNC_SCRIPT.name} failed (exit {completed.returncode}): {detail}"]
    return []


# ---------------------------------------------------------------------------
# Previews
# ---------------------------------------------------------------------------


def render_previews(results: list[ItemResult]) -> list[str]:
    """Write ``branding-mod/build/cosmetics-preview/<id>.png`` for model items."""

    import preview
    from PIL import Image

    PREVIEW_DIR.mkdir(parents=True, exist_ok=True)
    for result in results:
        if result.geometry is None or result.texture is None:
            continue
        geometry = result.geometry
        image = Image.new("RGBA", (geometry.texture_width, geometry.texture_height))
        image.putdata([pixel for row in result.texture for pixel in row])
        sheet = preview.render_sheet(result.item, geometry, image, result.preview_rows)
        path = PREVIEW_DIR / f"{result.item['id']}.png"
        sheet.convert("RGB").save(path, format="PNG", optimize=True)
        print(f"preview {path.relative_to(ROOT)}")
    return []


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="Build the Espectral cosmetics.")
    parser.add_argument("--check", action="store_true", help="validate only, write nothing")
    parser.add_argument("--item", action="append", metavar="ID", help="limit to one item id")
    parser.add_argument("--preview", action="store_true", help="render preview sheets")
    args = parser.parse_args(argv)

    try:
        base, folders = discover()
    except (BuildError, catalog_mod.CatalogError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2

    known = {folder.name for folder in folders}
    wanted = set(args.item or [])
    unknown = wanted - known
    if unknown:
        print(f"error: unknown item id(s) {sorted(unknown)}", file=sys.stderr)
        return 2

    results: list[ItemResult] = []
    for folder in folders:
        if wanted and folder.name not in wanted:
            continue
        try:
            item = load_item(folder, base["slots"] if folder.name in known else None)
        except ItemError as exc:
            result = ItemResult({"id": folder.name})
            result.errors.append(str(exc))
            results.append(result)
            continue
        result = ItemResult(item)
        try:
            if item["kind"] == "elytra":
                compile_elytra_item(folder, item, result)
            else:
                compile_model_item(folder, item, result, args.check)
        except pngio.TextureError as exc:
            result.errors.append(f"{item['id']}: {exc}")
        results.append(result)

    items = [result.item for result in results if result.valid]
    problems: list[str] = []
    if not wanted:
        problems.extend(catalog_mod.check_against_items(base, items))
    for result in results:
        problems.extend(result.errors)

    written: list[Path] = []
    unchanged: list[Path] = []
    removed: list[Path] = []

    if problems:
        pass
    elif args.check:
        problems.extend(check_generated(results, base, bool(wanted)))
    else:
        expected_models: set[Path] = set()
        expected_textures: set[Path] = set()
        for result in results:
            for path, data in result.outputs.items():
                write_if_changed(path, data, written, unchanged)
                if path.parent == MODEL_DIR:
                    expected_models.add(path)
                elif path.parent == TEXTURE_DIR:
                    expected_textures.add(path)

        if wanted:
            # A partial run only rewrites the item it names: the catalog and
            # the prune step are whole-project artefacts, and rebuilding them
            # from a single item would drop every other entry.
            print(f"partial run: catalog and stale-file pruning skipped")
        else:
            for path in prune(MODEL_DIR, expected_models, written):
                removed.append(path)
            for path in prune(TEXTURE_DIR, expected_textures, written):
                removed.append(path)

            catalog_bytes = catalog_mod.render(base, items).encode("utf-8")
            write_if_changed(CATALOG_FILE, catalog_bytes, written, unchanged)
            write_if_changed(SERVICE_CATALOG, catalog_bytes, written, unchanged)
            problems.extend(run_sync())

    if args.preview:
        problems.extend(render_previews(results))

    for path in written:
        print(f"wrote {path.relative_to(ROOT)}")
    for path in removed:
        print(f"removed {path.relative_to(ROOT)}")
    if unchanged:
        print(f"unchanged: {len(unchanged)} generated file(s)")
    if problems:
        print("", file=sys.stderr)
        for problem in problems:
            print(f"error: {problem}", file=sys.stderr)
        print(f"\n{len(problems)} problem(s)", file=sys.stderr)
        return 1

    print(f"ok: {catalog_mod.summary_text(base, items)}")
    return 0


def check_generated(results: list[ItemResult], base: dict, partial: bool) -> list[str]:
    """``--check``: the committed outputs must already match a fresh build."""

    problems: list[str] = []
    expected_models: set[Path] = set()
    expected_textures: set[Path] = set()
    for result in results:
        for path, data in result.outputs.items():
            if path.parent == MODEL_DIR:
                expected_models.add(path)
            elif path.parent == TEXTURE_DIR:
                expected_textures.add(path)
            if not path.is_file():
                problems.append(f"missing generated file {path.relative_to(ROOT)} — run the build")
            elif path.read_bytes() != data:
                problems.append(f"stale generated file {path.relative_to(ROOT)} — run the build")

    if partial:
        # The catalog covers every item and the directories hold the other
        # items' files, so a --item check cannot judge either.
        return problems

    for directory, expected in ((MODEL_DIR, expected_models), (TEXTURE_DIR, expected_textures)):
        if not directory.is_dir():
            continue
        for path in sorted(directory.iterdir()):
            if path.is_file() and path not in expected:
                problems.append(f"stale generated file {path.relative_to(ROOT)} — run the build")

    expected = catalog_mod.render(base, [r.item for r in results if r.valid]).encode("utf-8")
    for path in (CATALOG_FILE, SERVICE_CATALOG):
        if not path.is_file():
            problems.append(f"missing {path.relative_to(ROOT)} — run the build")
        elif path.read_bytes() != expected:
            problems.append(f"stale {path.relative_to(ROOT)} — run the build")
    return problems


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
