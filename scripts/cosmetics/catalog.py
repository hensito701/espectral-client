"""The generated cosmetics catalog.

Contract: ``docs/cosmetics-pipeline.md`` ("Catalog"). The catalog is derived
from ``catalog.base.json`` (schema, slots, exclusivity, emotes) plus every
``item.json``; its shape and one-object-per-line formatting are the ones the
engine, the mod and ``espectral-id`` already read, so the file stays
diff-friendly.

Consumers copy these exact bytes, so the writer must be deterministic:
``\\n`` line endings, one trailing newline (as the file that shipped before this
script existed), fixed key order.
"""

from __future__ import annotations

import json
from pathlib import Path

BASE_KEYS = frozenset({"schema", "slots", "exclusive", "emotes"})
EMOTE_KEYS = frozenset({"id", "requires", "name"})


class CatalogError(ValueError):
    """catalog.base.json is malformed; the message is build output."""


def load_base(path: Path) -> dict:
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except OSError as exc:
        raise CatalogError(f"catalog.base.json: cannot read ({exc})") from None
    except json.JSONDecodeError as exc:
        raise CatalogError(f"catalog.base.json: invalid JSON ({exc})") from None

    if not isinstance(raw, dict):
        raise CatalogError("catalog.base.json must contain an object")
    unknown = set(raw) - BASE_KEYS
    if unknown:
        raise CatalogError(f"catalog.base.json: unknown key(s) {sorted(unknown)}")
    required = {"schema", "slots", "exclusive", "emotes"}
    missing = required - set(raw)
    if missing:
        raise CatalogError(f"catalog.base.json: missing {sorted(missing)}")
    if raw["schema"] != 1:
        raise CatalogError(f"catalog.base.json: schema must be 1, found {raw['schema']!r}")

    slots = raw["slots"]
    if not isinstance(slots, list) or not slots or not all(isinstance(s, str) and s for s in slots):
        raise CatalogError("catalog.base.json: slots must be a non-empty array of names")
    if len(set(slots)) != len(slots):
        raise CatalogError("catalog.base.json: slots must be unique")

    exclusive = raw["exclusive"]
    if not isinstance(exclusive, list):
        raise CatalogError("catalog.base.json: exclusive must be an array of [slot, [ids]]")
    for rule in exclusive:
        if (
            not isinstance(rule, list)
            or len(rule) != 2
            or not isinstance(rule[0], str)
            or not isinstance(rule[1], list)
            or not all(isinstance(i, str) for i in rule[1])
        ):
            raise CatalogError("catalog.base.json: every exclusive rule is [slot, [ids]]")
        if rule[0] not in slots:
            raise CatalogError(f"catalog.base.json: exclusive rule names unknown slot '{rule[0]}'")

    emotes = raw["emotes"]
    if not isinstance(emotes, list):
        raise CatalogError("catalog.base.json: emotes must be an array")
    seen = set()
    for emote in emotes:
        if not isinstance(emote, dict):
            raise CatalogError("catalog.base.json: every emote must be an object")
        unknown = set(emote) - EMOTE_KEYS
        if unknown:
            raise CatalogError(f"catalog.base.json: emote has unknown key(s) {sorted(unknown)}")
        missing = EMOTE_KEYS - set(emote)
        if missing:
            raise CatalogError(f"catalog.base.json: emote is missing {sorted(missing)}")
        emote_id = emote["id"]
        if not isinstance(emote_id, str) or not emote_id:
            raise CatalogError("catalog.base.json: emote id must be a non-empty string")
        if emote_id in seen:
            raise CatalogError(f"catalog.base.json: duplicate emote '{emote_id}'")
        seen.add(emote_id)
        name = emote["name"]
        if not isinstance(name, dict) or set(name) != {"es", "en"}:
            raise CatalogError(f"catalog.base.json: emote '{emote_id}' needs name.es and name.en")
    return raw


def order_items(base: dict, items: list[dict]) -> list[dict]:
    """Items grouped in ``slots`` order, then by ``order``, then by id."""

    slot_index = {slot: index for index, slot in enumerate(base["slots"])}
    return sorted(items, key=lambda item: (slot_index[item["slot"]], item["order"], item["id"]))


def _compact(value: object) -> str:
    """JSON on one line with the shipped separators (``{"a":1,"b":[1,2]}``)."""

    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def render(base: dict, items: list[dict]) -> str:
    """The catalog document as the exact text this repository ships."""

    ordered = order_items(base, items)
    lines: list[str] = []
    lines.append("{ \"schema\": %d," % base["schema"])
    lines.append("  \"slots\": [%s]," % ",".join(json.dumps(slot, ensure_ascii=False) for slot in base["slots"]))
    if base["exclusive"]:
        rules = ", ".join(
            "[%s, [%s]]" % (json.dumps(slot, ensure_ascii=False), ",".join(json.dumps(i, ensure_ascii=False) for i in ids))
            for slot, ids in base["exclusive"]
        )
        lines.append("  \"exclusive\": [%s]," % rules)
    else:
        lines.append("  \"exclusive\": [],")
    lines.append("  \"items\": [")
    for index, item in enumerate(ordered):
        entry = {
            "id": item["id"],
            "slot": item["slot"],
            "requires": item["requires"],
            "name": item["name"],
        }
        suffix = "," if index + 1 < len(ordered) else ""
        lines.append("    %s%s" % (_compact(entry), suffix))
    lines.append("  ],")
    lines.append("  \"emotes\": [")
    emotes = base["emotes"]
    for index, emote in enumerate(emotes):
        entry = {"id": emote["id"], "requires": emote["requires"], "name": emote["name"]}
        suffix = "," if index + 1 < len(emotes) else ""
        lines.append("    %s%s" % (_compact(entry), suffix))
    lines.append("  ] }")
    return "\n".join(lines) + "\n"


def check_against_items(base: dict, items: list[dict]) -> list[str]:
    """Structural checks the catalog needs beyond per-item validation."""

    errors: list[str] = []
    ids = [item["id"] for item in items]
    if len(set(ids)) != len(ids):
        errors.append("duplicate item ids: " + ", ".join(sorted({i for i in ids if ids.count(i) > 1})))
    item_slots = {item["id"]: item["slot"] for item in items}
    for slot, targets in base["exclusive"]:
        for target in targets:
            if target not in item_slots:
                errors.append(f"exclusive rule for '{slot}' names unknown item '{target}'")
            elif item_slots[target] != slot:
                errors.append(f"exclusive rule for '{slot}' names '{target}', which is in '{item_slots[target]}'")
    return errors


def summary_text(base: dict, items: list[dict]) -> str:
    """One-line counts for the build log."""

    per_slot: dict[str, int] = {}
    for item in items:
        per_slot[item["slot"]] = per_slot.get(item["slot"], 0) + 1
    parts = ", ".join(f"{slot} {per_slot.get(slot, 0)}" for slot in base["slots"])
    return f"{len(items)} item(s) ({parts}), {len(base['emotes'])} emote(s)"


__all__ = [
    "CatalogError",
    "BASE_KEYS",
    "check_against_items",
    "load_base",
    "order_items",
    "render",
    "summary_text",
]
