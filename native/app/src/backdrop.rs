//! Static ambient backdrop (port of components/AmbientBackdrop.svelte).
//! FOUNDATION-OWNED.
//!
//! The Svelte backdrop animates three blurred radial "orbs" forever. Here the
//! orbs (at their initial positions) + the edge vignette are composited ONCE
//! into a quarter-resolution RGB image that the shell stretches with smooth
//! filtering — re-rendered only when the window size, theme, accent or ambient
//! hue changes. Cost: ~1 ms per change, 0% CPU while idle, ~250 KB of pixels.

use slint::{Color, ComponentHandle, Image, Rgb8Pixel, SharedPixelBuffer};

use crate::AppWindow;

/// Downscale factor of the backdrop texture.
const SCALE: f32 = 4.0;

#[derive(Clone, Copy)]
struct Rgba {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

impl Rgba {
    fn from_color(c: Color, a: f32) -> Self {
        Rgba { r: c.red() as f32 / 255.0, g: c.green() as f32 / 255.0, b: c.blue() as f32 / 255.0, a }
    }
    fn hsla(h: f32, s: f32, l: f32, a: f32) -> Self {
        let h = h.rem_euclid(360.0) / 360.0;
        let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
        let p = 2.0 * l - q;
        let f = |mut t: f32| {
            if t < 0.0 {
                t += 1.0
            }
            if t > 1.0 {
                t -= 1.0
            }
            if t < 1.0 / 6.0 {
                p + (q - p) * 6.0 * t
            } else if t < 0.5 {
                q
            } else if t < 2.0 / 3.0 {
                p + (q - p) * (2.0 / 3.0 - t) * 6.0
            } else {
                p
            }
        };
        Rgba { r: f(h + 1.0 / 3.0), g: f(h), b: f(h - 1.0 / 3.0), a }
    }
    fn transparent() -> Self {
        Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }
    }
}

/// Radial gradient with CSS (premultiplied) interpolation; offsets in 0..1 of the radius.
struct Orb {
    cx: f32,
    cy: f32,
    radius: f32,
    stops: Vec<(f32, Rgba)>,
}

impl Orb {
    /// Premultiplied (r, g, b, a) at distance ratio t.
    fn sample(&self, t: f32) -> (f32, f32, f32, f32) {
        let pm = |c: &Rgba| (c.r * c.a, c.g * c.a, c.b * c.a, c.a);
        if t <= self.stops[0].0 {
            return pm(&self.stops[0].1);
        }
        for w in self.stops.windows(2) {
            let (o0, c0) = (w[0].0, &w[0].1);
            let (o1, c1) = (w[1].0, &w[1].1);
            if t <= o1 {
                let k = if o1 > o0 { (t - o0) / (o1 - o0) } else { 1.0 };
                let (a, b) = (pm(c0), pm(c1));
                return (a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k, a.2 + (b.2 - a.2) * k, a.3 + (b.3 - a.3) * k);
            }
        }
        (0.0, 0.0, 0.0, 0.0)
    }
}

pub struct BackdropInputs {
    pub width: f32,
    pub height: f32,
    pub dark: bool,
    pub accent: Color,
    pub accent2: Color,
    pub gold: Color,
    pub hue: i32,
}

/// Render the backdrop texture (any thread).
pub fn render(inp: &BackdropInputs) -> SharedPixelBuffer<Rgb8Pixel> {
    let w = (inp.width.max(1.0) / SCALE).ceil() as u32;
    let h = (inp.height.max(1.0) / SCALE).ceil() as u32;
    let vw = inp.width / 100.0;
    let hue = inp.hue as f32;
    let dark = inp.dark;

    let s1 = (55.0 * vw).min(800.0);
    let s2 = (60.0 * vw).min(900.0);
    let s3 = (40.0 * vw).min(600.0);
    let orbs = [
        Orb {
            cx: -10.0 * vw + s1 / 2.0,
            cy: -10.0 * vw + s1 / 2.0,
            radius: s1 / 2.0,
            stops: vec![
                (0.0, Rgba::from_color(inp.accent, if dark { 0.12 } else { 0.08 })),
                (0.4, Rgba::hsla(hue, 0.8, 0.45, 0.12)),
                (0.7, Rgba::transparent()),
            ],
        },
        Orb {
            cx: inp.width + 10.0 * vw - s2 / 2.0,
            cy: inp.height + 15.0 * vw - s2 / 2.0,
            radius: s2 / 2.0,
            stops: vec![
                (0.0, Rgba::from_color(inp.accent2, if dark { 0.08 } else { 0.06 })),
                (0.45, Rgba::hsla(hue + 40.0, 0.75, 0.5, 0.08)),
                (0.7, Rgba::transparent()),
            ],
        },
        Orb {
            cx: inp.width * 0.75 - s3 / 2.0,
            cy: inp.height * 0.2 + s3 / 2.0,
            radius: s3 / 2.0,
            stops: vec![
                (0.0, Rgba::hsla(hue, 0.9, 0.55, 0.1)),
                (0.5, Rgba::from_color(inp.gold, 0.04)),
                (0.7, Rgba::transparent()),
            ],
        },
    ];
    let opacity = if dark { 0.85 } else { 0.45 };
    let bg =
        if dark { (6.0 / 255.0, 10.0 / 255.0, 20.0 / 255.0) } else { (244.0 / 255.0, 246.0 / 255.0, 251.0 / 255.0) };
    let vig = if dark {
        ((6.0 / 255.0, 10.0 / 255.0, 20.0 / 255.0), 0.6)
    } else {
        ((244.0 / 255.0, 246.0 / 255.0, 251.0 / 255.0), 0.45)
    };
    let (rx, ry) = (inp.width / 2.0 * std::f32::consts::SQRT_2, inp.height / 2.0 * std::f32::consts::SQRT_2);

    let mut acc: Vec<[f32; 3]> = vec![[0.0; 3]; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let px = (x as f32 + 0.5) * SCALE;
            let py = (y as f32 + 0.5) * SCALE;
            let mut c = bg;
            for orb in &orbs {
                let d = ((px - orb.cx).powi(2) + (py - orb.cy).powi(2)).sqrt() / orb.radius;
                if d >= 0.7 {
                    continue;
                }
                let (pr, pg, pb, pa) = orb.sample(d);
                let a = pa * opacity;
                if a <= 0.0 {
                    continue;
                }
                let (sr, sg, sb) = (pr / pa, pg / pa, pb / pa);
                let blend = |cb: f32, cs: f32| if dark { cb + cs - cb * cs } else { cb * cs };
                c = (
                    (1.0 - a) * c.0 + a * blend(c.0, sr),
                    (1.0 - a) * c.1 + a * blend(c.1, sg),
                    (1.0 - a) * c.2 + a * blend(c.2, sb),
                );
            }
            acc[(y * w + x) as usize] = [c.0, c.1, c.2];
        }
    }
    // CSS filter: blur(24px) on the orbs: 3 separable box passes ~ gaussian.
    let radius = (24.0 / SCALE).round() as i32;
    for _ in 0..3 {
        box_blur(&mut acc, w as i32, h as i32, radius);
    }
    let mut buf = SharedPixelBuffer::<Rgb8Pixel>::new(w, h);
    let pixels = buf.make_mut_slice();
    for y in 0..h {
        for x in 0..w {
            let px = (x as f32 + 0.5) * SCALE;
            let py = (y as f32 + 0.5) * SCALE;
            let a = acc[(y * w + x) as usize];
            let mut c = (a[0], a[1], a[2]);
            // vignette: transparent 50% -> color 100% of the farthest-corner ellipse
            let t = (((px - inp.width / 2.0) / rx).powi(2) + ((py - inp.height / 2.0) / ry).powi(2)).sqrt();
            if t > 0.5 {
                let va = ((t - 0.5) / 0.5).min(1.0) * vig.1;
                c = (
                    (1.0 - va) * c.0 + va * vig.0 .0,
                    (1.0 - va) * c.1 + va * vig.0 .1,
                    (1.0 - va) * c.2 + va * vig.0 .2,
                );
            }
            // tiny hash noise (±0.5 LSB) against banding when stretched
            let hash = (x.wrapping_mul(374_761_393) ^ y.wrapping_mul(668_265_263)).wrapping_mul(1_274_126_177);
            let dither = ((hash >> 24) as f32 / 255.0 - 0.5) / 255.0;
            let q = |v: f32| ((v + dither).clamp(0.0, 1.0) * 255.0).round() as u8;
            pixels[(y * w + x) as usize] = Rgb8Pixel { r: q(c.0), g: q(c.1), b: q(c.2) };
        }
    }
    buf
}

/// One horizontal + one vertical box-blur pass (edge-clamped).
fn box_blur(buf: &mut [[f32; 3]], w: i32, h: i32, r: i32) {
    if r <= 0 {
        return;
    }
    let mut tmp = buf.to_vec();
    let n = (2 * r + 1) as f32;
    for y in 0..h {
        for x in 0..w {
            let mut s = [0.0f32; 3];
            for k in -r..=r {
                let xx = (x + k).clamp(0, w - 1);
                let p = buf[(y * w + xx) as usize];
                s[0] += p[0];
                s[1] += p[1];
                s[2] += p[2];
            }
            tmp[(y * w + x) as usize] = [s[0] / n, s[1] / n, s[2] / n];
        }
    }
    for y in 0..h {
        for x in 0..w {
            let mut s = [0.0f32; 3];
            for k in -r..=r {
                let yy = (y + k).clamp(0, h - 1);
                let p = tmp[(yy * w + x) as usize];
                s[0] += p[0];
                s[1] += p[1];
                s[2] += p[2];
            }
            buf[(y * w + x) as usize] = [s[0] / n, s[1] / n, s[2] / n];
        }
    }
}

/// Wire AppWindow.backdrop-inputs-changed (UI thread, once).
pub fn init(ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.on_backdrop_inputs_changed(move |width, height, dark, accent, accent2, gold, hue| {
        let Some(ui) = weak.upgrade() else { return };
        let buf = render(&BackdropInputs { width, height, dark, accent, accent2, gold, hue });
        ui.set_backdrop_image(Image::from_rgb8(buf));
    });
    ui.invoke_refresh_backdrop();
}
