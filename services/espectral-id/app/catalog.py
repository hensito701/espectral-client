"""Cosmetics catalog loading and loadout validation.

The bundled catalog (app/cosmetics-catalog.json) is byte-identical to the
repository source of truth src/engine/cosmetics-catalog.json — deploy.sh
copies it at deploy time and tests/test_catalog_sync.py enforces equality.
"""
from __future__ import annotations

import json
import os

_CATALOG_PATH = os.path.join(os.path.dirname(__file__), "cosmetics-catalog.json")
_catalog_cache: dict | None = None


def load_catalog() -> dict:
    global _catalog_cache
    if _catalog_cache is None:
        with open(_CATALOG_PATH, "r", encoding="utf-8") as f:
            _catalog_cache = json.load(f)
    return _catalog_cache


def item_map(catalog: dict | None = None) -> dict[str, dict]:
    catalog = catalog or load_catalog()
    return {item["id"]: item for item in catalog.get("items", [])}


def exclusivity_map(catalog: dict | None = None) -> dict[tuple[str, str], set[str]]:
    """Map (slot, forbidden_item_id) -> set of slots it conflicts with.

    Catalog shape: [["cuello", ["manto_espectral","capucha_errante",...]]]
    meaning: an item in slot "cuello" may not be equipped together with any of
    the listed espalda items.
    """
    catalog = catalog or load_catalog()
    out: dict[tuple[str, str], set[str]] = {}
    for slot, forbidden_ids in catalog.get("exclusive", []):
        for item_id in forbidden_ids:
            items = item_map(catalog)
            other = items.get(item_id)
            if not other:
                continue
            out.setdefault((slot, item_id), set()).add(other["slot"])
    return out


def validate_loadout(loadout: dict, flags: list[str],
                     catalog: dict | None = None) -> dict:
    """Drop disallowed entries; return a clean {slot: itemId|null} loadout.

    Rules: known slots only, catalog items only, item must live in that slot,
    item requirement must be covered by flags, exclusivity pairs enforced.
    """
    catalog = catalog or load_catalog()
    items = item_map(catalog)
    slots = catalog.get("slots", [])
    flagset = set(flags or [])
    result = {slot: None for slot in slots}
    if not isinstance(loadout, dict):
        return result

    for slot in slots:
        item_id = loadout.get(slot)
        # Non-string values (lists, dicts, numbers) are treated as absent —
        # items.get() on an unhashable value would raise TypeError → 500.
        if not isinstance(item_id, str) or not item_id:
            continue
        item = items.get(item_id)
        if item is None or item.get("slot") != slot:
            continue
        req = item.get("requires")
        if req and req not in flagset:
            continue
        result[slot] = item_id

    # Exclusivity: a cuello item cannot coexist with a listed espalda item;
    # prefer keeping the espalda (cloak) and dropping the cuello item.
    excl = exclusivity_map(catalog)
    for (slot, forbidden_item_id), other_slots in excl.items():
        equipped = result.get(slot)
        if not equipped:
            continue
        for other_slot in other_slots:
            if result.get(other_slot) == forbidden_item_id:
                result[slot] = None
                break
    return result
