//! Deterministic procedural skins for tests, golden images and the example
//! (no third-party skin art is shipped). Not part of the stable API.

use crate::SkinTexture;

type Rgba = [u8; 4];

struct Canvas {
    px: Vec<u8>,
    w: u32,
}

impl Canvas {
    fn new(w: u32, h: u32) -> Canvas {
        Canvas { px: vec![0; (w * h * 4) as usize], w }
    }
    fn set(&mut self, x: u32, y: u32, c: Rgba) {
        let i = ((y * self.w + x) * 4) as usize;
        self.px[i..i + 4].copy_from_slice(&c);
    }
    fn fill(&mut self, x: u32, y: u32, w: u32, h: u32, c: Rgba) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.set(xx, yy, c);
            }
        }
    }
    /// Fills the 6 faces of a box net at (u, v) with dims (w, h, d), each with
    /// `c` shaded by a small per-face factor plus a 1-texel checker dither so
    /// texel edges are visible in renders.
    fn box_net(&mut self, u: u32, v: u32, w: u32, h: u32, d: u32, c: Rgba) {
        let faces = [
            (u + d, v, w, d, 1.08),             // top
            (u + d + w, v, w, d, 0.80),         // bottom
            (u, v + d, d, h, 0.92),             // right
            (u + d, v + d, w, h, 1.0),          // front
            (u + d + w, v + d, d, h, 0.96),     // left
            (u + 2 * d + w, v + d, w, h, 0.86), // back
        ];
        for (x, y, fw, fh, k) in faces {
            for yy in y..y + fh {
                for xx in x..x + fw {
                    let dither = if (xx + yy) % 2 == 0 { 1.0 } else { 0.93 };
                    self.set(xx, yy, tint(c, k * dither));
                }
            }
        }
    }
}

fn tint(c: Rgba, k: f32) -> Rgba {
    let f = |v: u8| ((v as f32 * k).round()).clamp(0.0, 255.0) as u8;
    [f(c[0]), f(c[1]), f(c[2]), c[3]]
}

const SKIN: Rgba = [214, 160, 122, 255];
const HAIR: Rgba = [74, 47, 27, 255];
const SHIRT: Rgba = [36, 168, 160, 255];
const PANTS: Rgba = [59, 76, 168, 255];
const SHOES: Rgba = [96, 96, 104, 255];
const WHITE: Rgba = [240, 240, 240, 255];
const IRIS: Rgba = [64, 96, 220, 255];
const MOUTH: Rgba = [140, 80, 60, 255];
const BAND: Rgba = [220, 60, 70, 255];
const GOLD: Rgba = [240, 190, 60, 255];

/// Base (first) layer of the fixture, rows 0..32 only (the legacy area).
fn paint_legacy_area(c: &mut Canvas) {
    // head (0,0) 8x8x8
    c.box_net(0, 0, 8, 8, 8, SKIN);
    c.fill(8, 0, 8, 8, HAIR); // top
    c.fill(24, 8, 8, 8, HAIR); // back
    c.fill(8, 8, 8, 2, HAIR); // fringe
    c.fill(0, 8, 8, 3, HAIR); // right side
    c.fill(16, 8, 8, 3, HAIR); // left side
    c.set(9, 12, WHITE);
    c.set(10, 12, IRIS);
    c.set(13, 12, IRIS);
    c.set(14, 12, WHITE);
    c.fill(11, 13, 2, 1, tint(SKIN, 0.8)); // nose
    c.fill(10, 14, 4, 1, MOUTH);
    // body (16,16) 8x12x4
    c.box_net(16, 16, 8, 12, 4, SHIRT);
    c.fill(20, 31, 8, 1, tint(PANTS, 0.9)); // belt line
                                            // right arm (40,16) 4x12x4: sleeve then skin
    c.box_net(40, 16, 4, 12, 4, SKIN);
    c.fill(40, 20, 16, 4, SHIRT);
    c.fill(44, 16, 4, 4, SHIRT); // shoulder top
                                 // right leg (0,16) 4x12x4
    c.box_net(0, 16, 4, 12, 4, PANTS);
    c.fill(0, 30, 16, 2, SHOES);
    c.fill(8, 16, 4, 4, SHOES); // sole
}

/// The fixture "Steve-like" skin, 64x64, with a second layer: a solid head
/// band, a translucent gold visor, a jacket belt and sleeve cuffs (overlay
/// texels use alpha 255 and 128 so both blending paths are exercised).
pub fn steve_rgba() -> Vec<u8> {
    let mut c = Canvas::new(64, 64);
    paint_legacy_area(&mut c);
    // left arm (32,48) / left leg (16,48): slightly different colours so
    // left / right are distinguishable.
    c.box_net(32, 48, 4, 12, 4, SKIN);
    c.fill(32, 52, 16, 4, tint(SHIRT, 0.9));
    c.fill(36, 48, 4, 4, tint(SHIRT, 0.9));
    c.box_net(16, 48, 4, 12, 4, tint(PANTS, 0.9));
    c.fill(16, 62, 16, 2, SHOES);
    c.fill(24, 48, 4, 4, SHOES);
    // hat layer (32,0): band all around at row 2 of the side faces
    c.fill(32, 10, 32, 1, BAND);
    c.fill(41, 12, 6, 1, [GOLD[0], GOLD[1], GOLD[2], 128]); // visor
    c.set(44, 3, [255, 255, 255, 128]); // translucent dot on the hat top
                                        // jacket (16,32): belt + buckle
    c.fill(16, 46, 24, 1, [40, 30, 20, 255]);
    c.set(23, 46, GOLD);
    c.set(24, 46, GOLD);
    // sleeve cuffs (40,32) and (48,48), rows 11 of the side faces
    c.fill(40, 47, 16, 1, WHITE);
    c.fill(48, 63, 16, 1, WHITE);
    c.px
}

/// The fixture skin as a texture.
pub fn steve() -> SkinTexture {
    SkinTexture::new(64, 64, steve_rgba()).expect("fixture is 64x64")
}

/// A legacy 64x32 version of the fixture (no second layer except the hat).
pub fn steve_legacy_rgba() -> Vec<u8> {
    let mut c = Canvas::new(64, 32);
    paint_legacy_area(&mut c);
    c.fill(32, 10, 32, 1, BAND);
    c.px
}

/// Every base-layer face painted with a unique flat colour (no shading
/// variation, no second layer): `face_color(part, face)`.
pub fn face_id_rgba() -> Vec<u8> {
    let mut c = Canvas::new(64, 64);
    for (p, &(u, v, w, h, d)) in PARTS.iter().enumerate() {
        for (f, (x, y, fw, fh)) in face_rects(u, v, w, h, d).into_iter().enumerate() {
            c.fill(x, y, fw, fh, face_color(p, f));
        }
    }
    c.px
}

/// Parts in [`face_id_rgba`] order: head, body, right arm, left arm, right leg, left leg.
pub const PARTS: [(u32, u32, u32, u32, u32); 6] = [
    (0, 0, 8, 8, 8),
    (16, 16, 8, 12, 4),
    (40, 16, 4, 12, 4),
    (32, 48, 4, 12, 4),
    (0, 16, 4, 12, 4),
    (16, 48, 4, 12, 4),
];

/// Face order of [`face_rects`].
pub const FACES: [&str; 6] = ["top", "bottom", "right", "front", "left", "back"];

/// Rects (x, y, w, h) of a box net: top, bottom, right (-x), front, left (+x), back.
pub fn face_rects(u: u32, v: u32, w: u32, h: u32, d: u32) -> [(u32, u32, u32, u32); 6] {
    [
        (u + d, v, w, d),
        (u + d + w, v, w, d),
        (u, v + d, d, h),
        (u + d, v + d, w, h),
        (u + d + w, v + d, d, h),
        (u + 2 * d + w, v + d, w, h),
    ]
}

/// Unique, well-separated colour of face `f` of part `p`.
pub fn face_color(p: usize, f: usize) -> Rgba {
    let i = (p * 6 + f) as u32;
    [(40 + (i % 4) * 60) as u8, (40 + ((i / 4) % 3) * 90) as u8, (40 + (i / 12) * 70) as u8, 255]
}
