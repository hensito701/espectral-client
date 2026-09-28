"""The bundled catalog must stay byte-identical to the repo source of truth."""
from __future__ import annotations

import os

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(
    os.path.dirname(os.path.abspath(__file__)))))
REPO_CATALOG = os.path.join(ROOT, "src", "engine", "cosmetics-catalog.json")
BUNDLED = os.path.join(os.path.dirname(os.path.dirname(
    os.path.abspath(__file__))), "app", "cosmetics-catalog.json")


def test_bundled_catalog_matches_repo_source():
    with open(REPO_CATALOG, "rb") as f:
        repo = f.read()
    with open(BUNDLED, "rb") as f:
        bundled = f.read()
    assert bundled == repo, (
        "app/cosmetics-catalog.json drifted from "
        "src/engine/cosmetics-catalog.json — re-copy it (deploy.sh does this)")
