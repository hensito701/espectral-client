//! Skin texture loading rules, ported from skinview-utils `loadSkinToCanvas`
//! (the code path skinview3d runs before every `loadSkin`):
//!
//! - legacy 64x32 skins are converted to the 1.8 (64x64) layout: the right
//!   arm / leg are copied, horizontally mirrored, into the left limb slots;
//!   the new bottom half (incl. every second-layer slot except the hat) stays
//!   transparent;
//! - "opaque" skins (no transparent pixel at all, typically a black
//!   background) get their second-layer areas cleared, otherwise the hat and
//!   jacket would render as solid boxes.
//!
//! HD skins (128x128, 256x128, ...) are handled by scaling every rect.

/// A rect in 64-space: x, y, w, h.
type Rect = (u32, u32, u32, u32);

/// Legacy -> 1.8: (src x, src y, w, h, dst x, dst y), each copy mirrored horizontally.
const LEGACY_COPIES: [(u32, u32, u32, u32, u32, u32); 12] = [
    (4, 16, 4, 4, 20, 48),   // leg top
    (8, 16, 4, 4, 24, 48),   // leg bottom
    (0, 20, 4, 12, 24, 52),  // leg outer
    (4, 20, 4, 12, 20, 52),  // leg front
    (8, 20, 4, 12, 16, 52),  // leg inner
    (12, 20, 4, 12, 28, 52), // leg back
    (44, 16, 4, 4, 36, 48),  // arm top
    (48, 16, 4, 4, 40, 48),  // arm bottom
    (40, 20, 4, 12, 40, 52), // arm outer
    (44, 20, 4, 12, 36, 52), // arm front
    (48, 20, 4, 12, 32, 52), // arm inner
    (52, 20, 4, 12, 44, 52), // arm back
];

/// Hat (helm) faces, cleared on opaque skins of both formats.
const HAT_AREAS: [Rect; 6] = [(40, 0, 8, 8), (48, 0, 8, 8), (32, 8, 8, 8), (40, 8, 8, 8), (48, 8, 8, 8), (56, 8, 8, 8)];

/// Second-layer body / limb faces, cleared on opaque 1.8 skins.
const LAYER2_AREAS: [Rect; 24] = [
    (4, 32, 4, 4),
    (8, 32, 4, 4),
    (0, 36, 4, 12),
    (4, 36, 4, 12),
    (8, 36, 4, 12),
    (12, 36, 4, 12),
    (20, 32, 8, 4),
    (28, 32, 8, 4),
    (16, 36, 4, 12),
    (20, 36, 8, 12),
    (28, 36, 4, 12),
    (32, 36, 8, 12),
    (44, 32, 4, 4),
    (48, 32, 4, 4),
    (40, 36, 4, 12),
    (44, 36, 4, 12),
    (48, 36, 4, 12),
    (52, 36, 12, 12),
    (4, 48, 4, 4),
    (8, 48, 4, 4),
    (0, 52, 4, 12),
    (4, 52, 4, 12),
    (8, 52, 4, 12),
    (12, 52, 4, 12),
];

const LEFT_ARM_LAYER2_AREAS: [Rect; 6] =
    [(52, 48, 4, 4), (56, 48, 4, 4), (48, 52, 4, 12), (52, 52, 4, 12), (56, 52, 4, 12), (60, 52, 4, 12)];

/// Converts any accepted input (`width` multiple of 64, square or 2:1) to a
/// square 1.8-layout texture. Returns the new rgba buffer (`width * width * 4`).
pub(crate) fn normalize(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let w = width as usize;
    let s = width / 64;
    let legacy = height * 2 == width;
    let mut out = vec![0u8; w * w * 4];
    out[..rgba.len()].copy_from_slice(rgba);
    if legacy {
        for &(sx, sy, cw, ch, dx, dy) in &LEGACY_COPIES {
            let (sx, sy, cw, ch, dx, dy) = (sx * s, sy * s, cw * s, ch * s, dx * s, dy * s);
            for y in 0..ch {
                for x in 0..cw {
                    let src = (((sy + y) * width + sx + x) * 4) as usize;
                    let dst = (((dy + y) * width + dx + (cw - 1 - x)) * 4) as usize;
                    let px: [u8; 4] = out[src..src + 4].try_into().unwrap();
                    out[dst..dst + 4].copy_from_slice(&px);
                }
            }
        }
    }
    // fixOpaqueSkin: look for any non-opaque pixel in the checked area.
    let check_rows = if legacy { w / 2 } else { w };
    let has_transparency = out[..check_rows * w * 4].chunks_exact(4).any(|p| p[3] != 255);
    if !has_transparency {
        let mut clear = |r: &Rect| {
            let (x0, y0, cw, ch) = (r.0 * s, r.1 * s, r.2 * s, r.3 * s);
            for y in y0..y0 + ch {
                let i = ((y * width + x0) * 4) as usize;
                out[i..i + (cw * 4) as usize].fill(0);
            }
        };
        HAT_AREAS.iter().for_each(&mut clear);
        if !legacy {
            LAYER2_AREAS.iter().for_each(&mut clear);
            LEFT_ARM_LAYER2_AREAS.iter().for_each(&mut clear);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(buf: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * w + x) * 4) as usize;
        buf[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn legacy_limbs_are_mirrored_into_left_slots() {
        let mut src = vec![0u8; 64 * 32 * 4];
        // right arm front (44..48, 20..32): a gradient along x
        for y in 20..32 {
            for x in 44..48 {
                let i = ((y * 64 + x) * 4) as usize;
                src[i..i + 4].copy_from_slice(&[(x - 44) as u8 * 50, 0, 0, 255]);
            }
        }
        let out = normalize(64, 32, &src);
        assert_eq!(out.len(), 64 * 64 * 4);
        // left arm front lives at (36..40, 52..64), mirrored
        assert_eq!(px(&out, 64, 36, 52)[0], 150);
        assert_eq!(px(&out, 64, 39, 52)[0], 0);
        assert_eq!(px(&out, 64, 37, 63)[0], 100);
        // second-layer slots of the new bottom half stay transparent
        assert_eq!(px(&out, 64, 52, 52)[3], 0);
    }

    #[test]
    fn opaque_skins_lose_their_second_layer() {
        let src = vec![255u8; 64 * 64 * 4];
        let out = normalize(64, 64, &src);
        assert_eq!(px(&out, 64, 42, 10)[3], 0, "hat front cleared");
        assert_eq!(px(&out, 64, 22, 40)[3], 0, "jacket front cleared");
        assert_eq!(px(&out, 64, 10, 10)[3], 255, "head kept");
    }

    #[test]
    fn transparent_skins_are_untouched() {
        let mut src = vec![255u8; 64 * 64 * 4];
        src[3] = 0;
        let out = normalize(64, 64, &src);
        assert_eq!(out, src);
    }

    #[test]
    fn hd_skins_scale_every_rect() {
        let src = vec![255u8; 128 * 128 * 4];
        let out = normalize(128, 128, &src);
        assert_eq!(px(&out, 128, 84, 20)[3], 0, "hat front cleared at 2x");
        assert_eq!(px(&out, 128, 20, 20)[3], 255);
    }
}
