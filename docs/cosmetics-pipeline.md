# Cosmetics pipeline

Supporter cosmetics are **data**: each item is a Blockbench-compatible model,
a pixel-art texture and a small metadata file. One build step checks the item
against the cosmetic rules, converts it into the runtime format and
regenerates the catalog. The mod loads every model item through one
generic renderer; nothing item-specific is written in Java.

```
branding-mod/cosmetics/
  catalog.base.json          schema, slots, exclusive rules, emotes (hand-edited)
  <item_id>/
    item.json                metadata + motion (hand-edited)
    model.geo.json           Bedrock geometry — open it in Blockbench (model items)
    texture.png              the texture Blockbench and the game use
    texture.txt              optional text pixel-art source; when present the
                             build regenerates texture.png from it
    frames/elytra_<n>.png    elytra skins only (kind "elytra")
```

`npm run cosmetics:build` (= `python scripts/cosmetics/build.py`) then writes:

| Output | Consumer |
|---|---|
| `branding-mod/src/main/resources/assets/espectral-menu/cosmetics/models/<id>.json` | mod runtime loader |
| `branding-mod/src/main/resources/assets/espectral-menu/textures/cosmetics/<id>.png` (elytra: `<id>.png` or `<id>_<n>.png`) | mod |
| `native/engine/data/cosmetics-catalog.json` | engine, then `scripts/sync-suite-registry.mjs` copies it into the mod and `services/espectral-id` |
| `branding-mod/build/cosmetics-preview/<id>.png` (with `--preview`) | human review: the item on a default player, several angles and poses |

The build is deterministic: unchanged sources produce byte-identical outputs.
`--check` validates only and exits non-zero on any error, `--item <id>` limits a
run to one item, `--preview` renders the preview sheets.

## item.json

```json
{
  "id": "bandana_roja",
  "kind": "model",
  "slot": "cabeza",
  "requires": "mvp",
  "order": 10,
  "name": { "es": "Pañuelo Rojo", "en": "Red Bandana" },
  "render": "cutout",
  "hide_with": "hat",
  "head_clearance_px": 0.6,
  "motion": {
    "tail_left":  [ { "type": "swing", "axis": "x", "rest": 10, "min": -5, "max": 70,
                      "stiffness": 0.18, "damping": 0.82, "drag": 40, "fall": 25, "turn": 0.6, "sway": 2 } ]
  }
}
```

| Field | Meaning |
|---|---|
| `id` | lowercase `[a-z0-9_]`, equals the folder name, unique |
| `kind` | `model` (geometry + texture) or `elytra` (texture skin for the vanilla elytra, no geometry) |
| `slot` | one of `catalog.base.json` `slots` |
| `requires` | entitlement flag (`mvp`) |
| `order` | catalog order inside the slot (ascending, ties by id) |
| `name` | `es` and `en`, both required |
| `render` | `cutout` (alpha 0 or opaque) or `translucent` (wings) |
| `hide_with` | vanilla skin-layer toggle that hides the item: `hat`, `cape` or `none`. Default: `cabeza`→`hat`, `espalda`→`cape`, otherwise `none` |
| `head_clearance_px` | how far above the head top the item may reach. Default `0.6` (the thickness of vanilla's hat layer, so a head-hugging bandana is legal). Above 0.6 only for ids on the build's exception list (`orejas_vulpinas`, `orejas_felinas`: up to 5) |
| `motion` | per-bone list of motion drivers (below) |

Elytra items carry `frames` (count, `1` = static) instead of geometry.

## model.geo.json (Bedrock geometry subset)

Standard Blockbench "Bedrock Entity" export, `format_version` `1.12.0`, exactly
one geometry. Units are model pixels (16 = 1 block), **Y up, origin at the
feet**, the wearer faces **−Z**, wearer's right is **−X** — the same frame as
the standard Bedrock player geometry (`geometry.humanoid.custom`).

```json
{
  "format_version": "1.12.0",
  "minecraft:geometry": [{
    "description": { "identifier": "geometry.espectral.bandana_roja",
                     "texture_width": 64, "texture_height": 32 },
    "bones": [
      { "name": "head", "pivot": [0, 24, 0] },
      { "name": "band", "parent": "head", "pivot": [0, 24, 0],
        "cubes": [ { "origin": [-4, 29, -4], "size": [8, 2, 8], "inflate": 0.55, "uv": [0, 0] } ] },
      { "name": "tail_left", "parent": "head", "pivot": [1, 29, 4.5], "rotation": [10, 0, 0],
        "cubes": [ { "origin": [0.5, 25, 4], "size": [1, 4, 1], "uv": [32, 0] } ] }
    ]
  }]
}
```

Rules:

- **Attachment bones.** Top-level bones (no `parent`) must be named after a
  player part and sit at that part's pivot: `head` `[0,24,0]`, `body`
  `[0,24,0]`, `right_arm` `[-5,22,0]`, `left_arm` `[5,22,0]`, `right_leg`
  `[-1.9,12,0]`, `left_leg` `[1.9,12,0]`. They carry no cubes and no rotation;
  everything else is their descendant. The attachment bone follows the real
  posed part (head yaw/pitch, crouch, emotes).
- Cubes: `origin` (min corner), `size`, `uv` as `[u, v]` box UV, optional
  `inflate` and `mirror`. Per-face UV objects are rejected. Sizes are whole
  pixels so every face lands on whole texels (inflate may be fractional).
- Bones: `name` unique, optional `parent`, `pivot`, `rotation` (degrees),
  optional `mirror`. No `locators`, `poly_mesh`, `texture_meshes` or
  `binding`.
- Texture size equals `texture_width`/`texture_height` (≤ 128 × 128).

### Conversion to the game's model space

Vanilla Java models use +Y down with the origin at the neck line; Bedrock
uses +Y up from the feet. X, Z and rotation angles are identical in both:

```
java_x = x                    java_z = z
java_y = 24 - y               (cube: java_min_y = 24 - (origin_y + size_y))
rotation: same three angles, degrees → radians, applied Z·Y·X (vanilla ModelPart order)
```

(This is the mapping GeckoLib uses, composed with the `scale(-1,-1,1)` every
vanilla living-entity renderer applies.) The runtime file stores everything
already converted: each bone's pivot relative to its parent bone's pivot,
each cube relative to its own bone's pivot. A calibration item proves the
mapping in game: something placed on the wearer's right (−X) and front (−Z)
in Blockbench must show on the wearer's right and on the face side.

## Texture

`texture.png` is RGBA. Every pixel is either fully transparent (alpha 0) or
at least alpha 200 (the minimum-opacity rule); `cutout` items should use only
0 or 255. Style rules from the owner (2026-10-25): flat pixel art at model
resolution, one texel per model pixel, no painted gradients or soft shading.

`texture.txt` — a text pixel-art source an LLM or a script can write:

```
size 64 32
palette
. transparent
R #b3262e
r #8e1d24
W #f2efe6
rows
RRRRRRRRrrrrrrrr................................................
...                (exactly `height` rows of exactly `width` characters)
```

Palette keys are single printable non-space characters; `.` is reserved for
transparent. Colours are `#rrggbb` (opaque) or `#rrggbbaa`.

## Motion drivers

Motion is declared per bone; angles in degrees around that bone's own axes
(the same axes as `rotation`, so Blockbench and the game agree). Drivers
read **only publicly visible pose state**: sneaking, sprinting, airborne /
falling speed, gliding, swimming, walk speed, head-yaw change, landing and
time. Nothing else is exposed to items, so no item can react to health,
damage or inventory. Several drivers on one bone add up.

All drivers rest at their neutral pose when cosmetics physics is off, under
reduced motion, or when the wearer is outside the nearest eight within 24
blocks.

| `type` | Fields | Behaviour |
|---|---|---|
| `swing` | `axis` (`x`/`z`), `rest`, `min`, `max`, `stiffness` (0.02–0.6), `damping` (0.5–0.98), `drag` (deg per block/tick of forward speed), `fall` (deg per block/tick of falling speed), `turn` (deg of kick per deg of body/head yaw change), `sway` (deg idle amplitude) | Damped spring toward `rest + drag·speed + fall·fallSpeed + sway`, kicked by yaw changes, clamped to `[min,max]`. A chain of child bones with their own `swing` lags naturally (tails, ribbons, panels hinged like the vanilla cape). |
| `react` | `sneak`, `sprint`, `airborne`, `glide`, `swim`: each `{ "x": deg, "y": deg, "z": deg }`, `rate` (0.05–1 per tick) | Smoothed additive offset while the state holds. |
| `head_turn` | `axis`, `gain` (deg per deg of head-yaw change per tick), `max`, `decay` (0.5–0.95) | Decaying impulse from head turns (ear twitch). |
| `land` | `axis`, `impulse` (deg), `decay` | Impulse when a fall ends (ear flop). |
| `flap` | `axis`, `closed`, `open` (deg), `beat` (signed deg amplitude; mirror it on the opposite wing), `beat_speed` (rad per tick) | Open amount 0..1: open while falling or gliding, half while sprinting, folded while sneaking; beats while open. Angle = `closed + (open − closed)·amount + beat·sin(t·beat_speed)·amount`. |

The build checks the footprint at the rest pose **and** at every reachable
extreme of each driver (`min`/`max`, the react offsets, `±max`, `impulse`,
`closed`/`open ± beat`).

## Checks (`--check` fails the build)

1. Schema: required fields, known slot / kind / render / hide_with, `id` =
   folder, unique ids, names in both languages.
2. Geometry: attachment bones as listed above, unique bone names, known
   parents, no cycles, box UV only, whole-pixel cube sizes, UV boxes inside
   the texture, ≤ 64 cubes per item.
3. Texture: size matches geometry, alpha is 0 or ≥ 200 everywhere,
   `texture.txt` (if present) parses and matches `texture.png`.
4. Footprint (vanilla model space, part frames at rest): nothing more than
   `head_clearance_px` above the head top (y = −8); nothing below the knees
   (y = 18); |x| ≤ 12.5 (shoulders plus one block). Checked at rest and at
   every driver extreme.
5. Motion: drivers reference existing bones, ranges are sane (`min ≤ rest ≤
   max`, coefficients inside the listed bounds).

## Catalog

`native/engine/data/cosmetics-catalog.json` is generated from
`catalog.base.json` plus every `item.json` (`id`, `slot`, `requires`, `name`,
items grouped in `slots` order then by `order`). Its shape is unchanged, so
the engine, the mod and `espectral-id` keep reading it as before. After a
build, `node scripts/sync-suite-registry.mjs` copies it into the mod;
`services/espectral-id` picks it up on its next deploy (its loadout check
drops items it does not know).

## Runtime (mod)

- `cosmetics/render/ItemModels` loads `assets/espectral-menu/cosmetics/models/<id>.json`
  for every `model` item in the catalog, bakes one `ModelPart` tree per
  attachment bone and caches it.
- **Hot reload:** with `-Despectral.cosmetics.dir=<folder>` (a folder of
  compiled `models/<id>.json` + `textures/<id>.png`, e.g. the build output),
  the mod rechecks file times once per second and rebuilds changed items.
  It only replaces assets of items the catalog already lists and grants
  nothing.
- **Dev preview:** `-Despectral.cosmetics.preview=<id,...>` (development
  environment only) equips items on your own player without a grant.
- `CosmeticLayer` draws each equipped model item under its attachment parts
  (skipped when `hide_with` says the vanilla layer is hidden); the motion
  drivers are stepped per wearer on the client tick in `WearerSims`.

### Compiled model file (`cosmetics/models/<id>.json`)

Written by the build, read by the mod. All positions are already in vanilla
model space (pixels, +Y down) and rotations in radians, so the loader only
feeds them to `CubeListBuilder` / `PartPose`.

```json
{
  "schema": 1,
  "id": "bandana_roja",
  "render": "cutout",
  "hide_with": "hat",
  "texture": { "width": 64, "height": 32 },
  "parts": [
    { "part": "head",
      "bones": [
        { "name": "band", "pivot": [0, 0, 0], "rotation": [0, 0, 0],
          "cubes": [ { "from": [-4, -7, -4], "size": [8, 2, 8], "uv": [0, 0], "inflate": 0.55, "mirror": false } ],
          "children": [],
          "motion": [] }
      ] }
  ]
}
```

- `parts[].part`: `head`, `body`, `right_arm`, `left_arm`, `right_leg`, `left_leg`
  (the vanilla `PlayerModel` fields).
- Bone `pivot`: relative to the parent bone's pivot; for a bone directly
  under the attachment, relative to the attachment part's pivot.
  `rotation`: radians, applied in vanilla `ModelPart` order.
- Cube `from`: min corner relative to its own bone's pivot; `size`, `uv`,
  `inflate`, `mirror` map 1:1 to `CubeListBuilder.texOffs(u,v).mirror(m)
  .addBox(from, size, CubeDeformation(inflate))`.
- `motion`: the item's drivers for that bone, verbatim from `item.json`
  (angles in degrees, converted by the runtime). Driver angles add to the
  bone's rest `rotation` on the driver's `axis`.
- Texture: `assets/espectral-menu/textures/cosmetics/<id>.png`.

## Adding an item

1. `mkdir branding-mod/cosmetics/<id>`; write `item.json`.
2. Model it in Blockbench (File → New → Bedrock Entity, import the player
   reference) or write `model.geo.json` by hand; paint `texture.png` or write
   `texture.txt`.
3. `npm run cosmetics:build -- --item <id> --preview` until it passes; look at
   `branding-mod/build/cosmetics-preview/<id>.png`.
4. Dev check in game: `-Despectral.cosmetics.preview=<id>` and
   `-Despectral.cosmetics.dir=…`.
5. Full build + `node scripts/sync-suite-registry.mjs`, owner approval, then
   `npm run deploy:mod`.
