"""``item.json``: schema validation, motion-driver checks and extreme poses.

Contract: ``docs/cosmetics-pipeline.md`` ("item.json", "Motion drivers",
"Checks" rules 1 and 5).
"""

from __future__ import annotations

import json
import math
import re
from pathlib import Path

ID_PATTERN = re.compile(r"^[a-z0-9_]+$")

KINDS = ("model", "elytra")
RENDERS = ("cutout", "translucent")
HIDE_WITH = ("hat", "cape", "none")

#: Default ``head_clearance_px``: the vanilla hat layer's thickness.
DEFAULT_CLEARANCE = 0.6
#: Items allowed past the default, with their ceiling (the recorded ear exception).
CLEARANCE_EXCEPTIONS: dict[str, float] = {"orejas_vulpinas": 5.0, "orejas_felinas": 5.0}

DRIVER_TYPES = ("swing", "react", "head_turn", "land", "flap")
REACT_STATES = ("sneak", "sprint", "airborne", "glide", "swim")

#: Field bounds from the driver table: ``(name, low, high)``.
BOUNDED = {
    "swing": (("stiffness", 0.02, 0.6), ("damping", 0.5, 0.98)),
    "react": (("rate", 0.05, 1.0),),
    "head_turn": (("decay", 0.5, 0.95),),
    "land": (("decay", 0.5, 0.95),),
    "flap": (),
}
#: Keys each driver accepts; anything else is a typo the build must catch.
DRIVER_KEYS = {
    "swing": frozenset({"type", "axis", "rest", "min", "max", "stiffness", "damping", "drag", "fall", "turn", "sway"}),
    "react": frozenset({"type", "rate", *REACT_STATES}),
    "head_turn": frozenset({"type", "axis", "gain", "max", "decay"}),
    "land": frozenset({"type", "axis", "impulse", "decay"}),
    "flap": frozenset({"type", "axis", "closed", "open", "beat", "beat_speed"}),
}
#: Axes each single-axis driver accepts.
DRIVER_AXES = {
    "swing": ("x", "z"),
    "head_turn": ("x", "y", "z"),
    "land": ("x", "y", "z"),
    "flap": ("x", "y", "z"),
}

ITEM_KEYS = frozenset(
    {
        "id",
        "kind",
        "slot",
        "requires",
        "order",
        "name",
        "render",
        "hide_with",
        "head_clearance_px",
        "motion",
        "frames",
    }
)


class ItemError(ValueError):
    """An item source is invalid; the message is build output."""


def load_json(path: Path) -> object:
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as exc:
        raise ItemError(f"{path.name}: cannot read ({exc})") from None
    try:
        return json.loads(text)
    except json.JSONDecodeError as exc:
        raise ItemError(f"{path.name}: invalid JSON ({exc})") from None


def load_item(folder: Path, slots: list[str] | None = None) -> dict:
    """Read and validate ``<folder>/item.json``; returns the item with defaults filled."""

    raw = load_json(folder / "item.json")
    return validate_item(raw, folder.name, slots)


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise ItemError(message)


def _number(value: object, what: str) -> float:
    _require(not isinstance(value, bool) and isinstance(value, (int, float)), f"{what} must be a number")
    number = float(value)  # type: ignore[arg-type]
    _require(math.isfinite(number), f"{what} must be finite")
    return number


def _integer(value: object, what: str) -> int:
    _require(not isinstance(value, bool) and isinstance(value, int), f"{what} must be an integer")
    return value  # type: ignore[return-value]


def _string(value: object, what: str) -> str:
    _require(isinstance(value, str) and value != "", f"{what} must be a non-empty string")
    return value  # type: ignore[return-value]


def validate_item(raw: object, folder: str, slots: list[str] | None = None) -> dict:
    """Validate one item document and return a normalised copy."""

    _require(isinstance(raw, dict), f"{folder}: item.json must contain an object")
    unknown = set(raw) - ITEM_KEYS
    _require(not unknown, f"{folder}: unknown item field(s) {sorted(unknown)}")

    item_id = _string(raw.get("id"), f"{folder}: id")
    _require(ID_PATTERN.match(item_id) is not None, f"{folder}: id '{item_id}' must be [a-z0-9_]")
    _require(item_id == folder, f"{folder}: id '{item_id}' must equal the folder name")

    kind = _string(raw.get("kind"), f"{folder}: kind")
    _require(kind in KINDS, f"{folder}: kind must be one of {list(KINDS)}, found '{kind}'")

    slot = _string(raw.get("slot"), f"{folder}: slot")
    if slots is not None:
        _require(slot in slots, f"{folder}: slot '{slot}' is not in catalog slots {slots}")
    requires = _string(raw.get("requires"), f"{folder}: requires")
    _require(ID_PATTERN.match(requires) is not None, f"{folder}: requires '{requires}' must be [a-z0-9_]")
    order = _integer(raw.get("order"), f"{folder}: order")

    name = raw.get("name")
    _require(isinstance(name, dict), f"{folder}: name must be an object")
    _require(set(name) == {"es", "en"}, f"{folder}: name needs exactly 'es' and 'en'")
    _require(all(isinstance(v, str) and v.strip() for v in name.values()), f"{folder}: both names are required")
    names = {"es": name["es"].strip(), "en": name["en"].strip()}

    default_render = "cutout"
    render = raw.get("render", default_render)
    _require(render in RENDERS, f"{folder}: render must be one of {list(RENDERS)}, found {render!r}")

    default_hide = "hat" if slot == "cabeza" else "cape" if slot == "espalda" else "none"
    hide_with = raw.get("hide_with", default_hide)
    _require(hide_with in HIDE_WITH, f"{folder}: hide_with must be one of {list(HIDE_WITH)}, found {hide_with!r}")

    clearance = _number(raw.get("head_clearance_px", DEFAULT_CLEARANCE), f"{folder}: head_clearance_px")
    _require(clearance >= 0.0, f"{folder}: head_clearance_px must not be negative")
    limit = CLEARANCE_EXCEPTIONS.get(item_id, DEFAULT_CLEARANCE)
    _require(
        clearance <= limit + 1e-9,
        f"{folder}: head_clearance_px {clearance:g} exceeds {limit:g}"
        + ("" if item_id in CLEARANCE_EXCEPTIONS else " (only the ear items are exempt from the 0.6 px default)"),
    )

    motion = raw.get("motion", {})
    _require(isinstance(motion, dict), f"{folder}: motion must be an object")
    motion = {bone: validate_drivers(bone, drivers, folder) for bone, drivers in motion.items()}

    item = {
        "id": item_id,
        "kind": kind,
        "slot": slot,
        "requires": requires,
        "order": order,
        "name": names,
        "render": render,
        "hide_with": hide_with,
        "head_clearance_px": clearance,
        "motion": motion,
    }

    if kind == "elytra":
        _require("frames" in raw, f"{folder}: an elytra item needs 'frames' (1 = static)")
        frames = _integer(raw["frames"], f"{folder}: frames")
        _require(1 <= frames <= 64, f"{folder}: frames must be 1..64, found {frames}")
        _require(not motion, f"{folder}: elytra items have no geometry, so no motion")
        item["frames"] = frames
    else:
        _require("frames" not in raw, f"{folder}: only elytra items carry 'frames'")
    return item


def validate_drivers(bone: str, drivers: object, folder: str) -> list[dict]:
    """Validate one bone's driver list and return it verbatim (plus declared defaults)."""

    _require(isinstance(drivers, list) and drivers, f"{folder}: motion['{bone}'] must be a non-empty array")
    out: list[dict] = []
    for index, driver in enumerate(drivers):
        where = f"{folder}: motion['{bone}'][{index}]"
        _require(isinstance(driver, dict), f"{where} must be an object")
        kind = driver.get("type")
        _require(kind in DRIVER_TYPES, f"{where}: unknown driver type {kind!r}")
        allowed = DRIVER_KEYS[kind]
        unknown = set(driver) - allowed
        _require(not unknown, f"{where}: unsupported field(s) {sorted(unknown)} for a '{kind}' driver")

        if kind in DRIVER_AXES:
            axis = driver.get("axis", "x")
            _require(axis in DRIVER_AXES[kind], f"{where}: axis must be one of {list(DRIVER_AXES[kind])}")
        for field, low, high in BOUNDED[kind]:
            if field in driver:
                value = _number(driver[field], f"{where}.{field}")
                _require(low - 1e-9 <= value <= high + 1e-9, f"{where}: {field} {value:g} outside {low:g}..{high:g}")

        if kind == "swing":
            for field in ("rest", "min", "max"):
                _require(field in driver, f"{where}: swing needs '{field}'")
            rest = _number(driver["rest"], f"{where}.rest")
            low = _number(driver["min"], f"{where}.min")
            high = _number(driver["max"], f"{where}.max")
            _require(low <= rest + 1e-9 and rest <= high + 1e-9, f"{where}: needs min <= rest <= max, got {low:g} / {rest:g} / {high:g}")
            for field in ("drag", "fall", "turn", "sway"):
                if field in driver:
                    _number(driver[field], f"{where}.{field}")
        elif kind == "react":
            _require(
                any(state in driver for state in REACT_STATES),
                f"{where}: a react driver needs at least one of {list(REACT_STATES)}",
            )
            for state in REACT_STATES:
                if state not in driver:
                    continue
                offsets = driver[state]
                _require(isinstance(offsets, dict), f"{where}.{state} must be an object")
                unknown_offsets = set(offsets) - {"x", "y", "z"}
                _require(not unknown_offsets, f"{where}.{state}: unsupported key(s) {sorted(unknown_offsets)}")
                for axis in ("x", "y", "z"):
                    if axis in offsets:
                        _number(offsets[axis], f"{where}.{state}.{axis}")
        elif kind == "head_turn":
            _require("gain" in driver, f"{where}: head_turn needs 'gain'")
            _number(driver["gain"], f"{where}.gain")
            maximum = _number(driver.get("max", 30.0), f"{where}.max")
            _require(maximum > 0.0, f"{where}: max must be positive")
        elif kind == "land":
            _require("impulse" in driver, f"{where}: land needs 'impulse'")
            _number(driver["impulse"], f"{where}.impulse")
        elif kind == "flap":
            for field in ("closed", "open"):
                _require(field in driver, f"{where}: flap needs '{field}'")
                _number(driver[field], f"{where}.{field}")
            if "beat" in driver:
                _number(driver["beat"], f"{where}.beat")
            if "beat_speed" in driver:
                speed = _number(driver["beat_speed"], f"{where}.beat_speed")
                _require(speed >= 0.0, f"{where}: beat_speed must not be negative")
        out.append(dict(driver))
    return out


# ---------------------------------------------------------------------------
# Driver extremes
# ---------------------------------------------------------------------------


def _offset(driver: dict, state: str) -> tuple[float, float, float]:
    offsets = driver.get(state) or {}
    return (
        float(offsets.get("x", 0.0)),
        float(offsets.get("y", 0.0)),
        float(offsets.get("z", 0.0)),
    )


def neutral_pose(item: dict) -> dict[str, tuple[float, float, float]]:
    """The pose every driver rests at when cosmetics physics is off.

    ``swing`` rests at its ``rest`` angle, ``flap`` at ``closed``, and the
    impulse/state drivers at zero — exactly what ``MotionDriver.neutral``
    writes, so the footprint and the preview see the same pose the game shows
    a wearer outside the simulation set.
    """

    pose: dict[str, tuple[float, float, float]] = {}
    for bone, drivers in item["motion"].items():
        x = y = z = 0.0
        for driver in drivers:
            axis = driver.get("axis", "x")
            if driver["type"] == "swing":
                angle = float(driver["rest"])
            elif driver["type"] == "flap":
                angle = float(driver["closed"])
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


def _axis_vector(axis: str, angle: float) -> tuple[float, float, float]:
    if axis == "y":
        return (0.0, angle, 0.0)
    if axis == "z":
        return (0.0, 0.0, angle)
    return (angle, 0.0, 0.0)


def driver_extremes(drivers: list[dict]) -> list[tuple[str, tuple[float, float, float]]]:
    """Every reachable angular extreme one bone's drivers can add, with a label."""

    out: list[tuple[str, tuple[float, float, float]]] = []
    for driver in drivers:
        kind = driver["type"]
        axis = driver.get("axis", "x")
        if kind == "swing":
            out.append((f"swing min {driver['min']:g}", _axis_vector(axis, float(driver["min"]))))
            out.append((f"swing max {driver['max']:g}", _axis_vector(axis, float(driver["max"]))))
        elif kind == "react":
            for state in REACT_STATES:
                if state in driver:
                    out.append((f"react {state}", _offset(driver, state)))
        elif kind == "head_turn":
            maximum = float(driver.get("max", 30.0))
            out.append((f"head_turn {maximum:g}", _axis_vector(axis, maximum)))
            out.append((f"head_turn -{maximum:g}", _axis_vector(axis, -maximum)))
        elif kind == "land":
            impulse = float(driver["impulse"])
            out.append((f"land impulse {impulse:g}", _axis_vector(axis, impulse)))
        elif kind == "flap":
            closed = float(driver["closed"])
            opened = float(driver["open"])
            beat = float(driver.get("beat", 0.0))
            for label, angle in (
                ("flap closed", closed),
                ("flap open + beat", opened + beat),
                ("flap open - beat", opened - beat),
            ):
                out.append((label, _axis_vector(axis, angle)))
    return out
