//! espectral-skin3d — native Minecraft skin viewer (replaces skinview3d / three.js).
//!
//! OWNER: the skin-renderer agent (see `native/app/UI-PORTING.md`).
//!
//! A pure-Rust CPU renderer (no GPU, no dependencies) that turns a Minecraft
//! skin texture (64x64, legacy 64x32 or HD multiples, classic or slim arms,
//! with the second / overlay layer) into an RGBA frame:
//!
//! - [`SkinTexture::new`] applies skinview3d's loading rules (legacy -> 1.8
//!   layout, opaque-skin fix), see `texture.rs`;
//! - `model.rs` builds the player (head, body, 2 arms, 2 legs; inner boxes +
//!   inflated outer boxes) with skinview3d's exact geometry and UV mapping,
//!   posed by a [`Pose`] (yaw / pitch, walk cycle, head bob, idle arm sway);
//! - `raster.rs` projects it with skinview3d's camera (50° vertical FOV,
//!   same distance formula) and rasterizes it: z-buffered, perspective-correct,
//!   nearest-neighbour texturing, simple directional shading, alpha-blended
//!   double-sided second layer, optional 2x2 supersampling.
//!
//! The UI calls it from a Slint pure callback on the UI thread whenever an
//! input changes, so a ~300x400 frame renders in well under a millisecond in
//! release builds (see `examples/render_skins.rs`, which also benchmarks).

mod math;
mod model;
mod raster;
mod texture;

#[doc(hidden)]
pub mod fixtures;

use std::sync::Mutex;

use math::{v3, Xf, M3};
use model::Quad;
use raster::{Projection, Raster, Tex};

/// Arm model of a skin (`SkinVariant` in the engine: "classic" | "slim").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArmModel {
    #[default]
    Classic,
    Slim,
}

impl ArmModel {
    /// Engine / JSON spelling ("slim" -> Slim, anything else -> Classic).
    pub fn from_variant(v: &str) -> Self {
        if v.eq_ignore_ascii_case("slim") {
            ArmModel::Slim
        } else {
            ArmModel::Classic
        }
    }
}

/// Decoded skin texture, straight RGBA8 (not premultiplied), row-major, always
/// in the square 1.8 layout (legacy skins are converted by [`SkinTexture::new`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkinTexture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl SkinTexture {
    /// `None` unless the buffer matches the size and the texture is 64x64 or 64x32
    /// (or an HD multiple of those). Legacy 64x32 skins come back as 64x64
    /// (mirrored left limbs) and opaque skins lose their second layer, exactly
    /// like skinview3d's `loadSkinToCanvas`.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Option<Self> {
        let ok_shape = width >= 64 && width % 64 == 0 && (height == width || height * 2 == width);
        if !ok_shape || rgba.len() != (width as usize) * (height as usize) * 4 {
            return None;
        }
        let rgba = texture::normalize(width, height, &rgba);
        Some(SkinTexture { width, height: width, rgba })
    }

    /// Texel at skin-space (64-wide) coordinates, scaled for HD skins.
    pub fn texel(&self, x: u32, y: u32) -> [u8; 4] {
        let s = self.width / 64;
        let (x, y) = (x * s, y * s);
        if x >= self.width || y >= self.height {
            return [0, 0, 0, 0];
        }
        let i = ((y * self.width + x) * 4) as usize;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }
}

/// Figure orientation + animation state for one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    /// Turn of the player around the vertical axis, degrees (0 = facing the
    /// camera, positive = his front turns to the viewer's right).
    pub yaw_deg: f32,
    /// Camera elevation, degrees (positive = looking from above).
    pub pitch_deg: f32,
    /// Walk cycle phase, radians (skinview3d WalkingAnimation `t`).
    pub walk_phase: f32,
    /// Walk swing amplitude of arms and legs, radians (0 = standing, 0.5 = walking).
    pub arm_swing: f32,
    /// Head bob amount while walking (0 = none, 1 = skinview3d's).
    pub head_bob: f32,
    /// Idle arm sway phase, radians (skinview3d IdleAnimation `t` = 2 * seconds).
    pub idle_phase: f32,
    /// How far the arms rest away from the body (1 = skinview3d idle pose, 0 = straight).
    pub arm_rest: f32,
}

impl Default for Pose {
    fn default() -> Self {
        Pose {
            yaw_deg: 0.0,
            pitch_deg: 0.0,
            walk_phase: 0.0,
            arm_swing: 0.0,
            head_bob: 0.0,
            idle_phase: 0.0,
            arm_rest: 1.0,
        }
    }
}

impl Pose {
    /// skinview3d's IdleAnimation at `time_s` seconds, seen from (yaw, pitch).
    pub fn idle(yaw_deg: f32, pitch_deg: f32, time_s: f32) -> Pose {
        Pose { yaw_deg, pitch_deg, idle_phase: time_s * 2.0, ..Pose::default() }
    }

    /// skinview3d's WalkingAnimation (with head bobbing) at `time_s` seconds.
    pub fn walking(yaw_deg: f32, pitch_deg: f32, time_s: f32) -> Pose {
        let t = time_s * 8.0;
        Pose { yaw_deg, pitch_deg, walk_phase: t, arm_swing: 0.5, head_bob: 1.0, idle_phase: t, arm_rest: 1.0 }
    }
}

/// Everything one frame needs besides the texture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scene {
    /// Output size in physical pixels.
    pub width: u32,
    pub height: u32,
    pub model: ArmModel,
    pub pose: Pose,
    /// 1.0 = skinview3d's default framing (its `zoom: 0.9`).
    pub zoom: f32,
    /// Draw the second (overlay) layer.
    pub overlay: bool,
    /// 2x2 supersampling (smoother edges, ~4x the fill cost).
    pub antialias: bool,
    /// Directional shading; `false` = flat texel colours.
    pub shading: bool,
}

impl Scene {
    pub fn new(width: u32, height: u32, model: ArmModel, pose: Pose) -> Scene {
        Scene { width, height, model, pose, zoom: 1.0, overlay: true, antialias: false, shading: true }
    }
}

/// Camera + pose for one frame (the app-facing contract, see UI-PORTING.md).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderParams {
    /// Output size in physical pixels.
    pub width: u32,
    pub height: u32,
    /// Rotation around the vertical axis, degrees (0 = facing the camera).
    pub yaw_deg: f32,
    /// Rotation around the horizontal axis, degrees (positive = look from above).
    pub pitch_deg: f32,
    /// 1.0 = the whole player fits the frame (skinview3d default framing).
    pub zoom: f32,
    pub model: ArmModel,
    /// Idle animation clock in seconds (skinview3d IdleAnimation); 0 = rest pose.
    pub time_s: f32,
    /// Draw the second (overlay) layer.
    pub overlay: bool,
}

impl Default for RenderParams {
    fn default() -> Self {
        RenderParams {
            width: 280,
            height: 340,
            yaw_deg: 0.0,
            pitch_deg: 0.0,
            zoom: 1.0,
            model: ArmModel::Classic,
            time_s: 0.0,
            overlay: true,
        }
    }
}

impl From<&RenderParams> for Scene {
    fn from(p: &RenderParams) -> Scene {
        Scene {
            width: p.width,
            height: p.height,
            model: p.model,
            pose: Pose::idle(p.yaw_deg, p.pitch_deg, p.time_s),
            zoom: p.zoom,
            overlay: p.overlay,
            antialias: false,
            shading: true,
        }
    }
}

/// Rendered RGBA8 frame (straight alpha), row-major, `width * height * 4` bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl RgbaFrame {
    pub fn transparent(width: u32, height: u32) -> Self {
        RgbaFrame { width, height, pixels: vec![0; (width as usize) * (height as usize) * 4] }
    }

    /// Pixel at (x, y) (transparent outside the frame).
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.width || y >= self.height {
            return [0; 4];
        }
        let i = ((y * self.width + x) * 4) as usize;
        [self.pixels[i], self.pixels[i + 1], self.pixels[i + 2], self.pixels[i + 3]]
    }
}

/// skinview3d's camera: vertical field of view, degrees.
pub const FOV_DEG: f32 = 50.0;
/// skinview3d's default `zoom`.
const BASE_ZOOM: f32 = 0.9;
/// Light: a little above and to the left of the camera (view space).
const LIGHT: (f32, f32, f32) = (-0.35, 0.5, 1.0);
const AMBIENT: f32 = 0.7;
const DIFFUSE: f32 = 0.32;

/// Reusable renderer state (z-buffer, quad list, supersampling buffer).
/// Keep one around to render without per-frame allocations.
#[derive(Default)]
pub struct Rasterizer {
    depth: Vec<f32>,
    quads: Vec<Quad>,
    big: Option<RgbaFrame>,
}

impl Rasterizer {
    pub fn new() -> Rasterizer {
        Rasterizer::default()
    }

    /// Renders `scene` into a new frame.
    pub fn render(&mut self, skin: &SkinTexture, scene: &Scene) -> RgbaFrame {
        let mut out = RgbaFrame::transparent(scene.width, scene.height);
        self.render_into(skin, scene, &mut out);
        out
    }

    /// Renders `scene` into `out` (resized to the scene size).
    pub fn render_into(&mut self, skin: &SkinTexture, scene: &Scene, out: &mut RgbaFrame) {
        if !scene.antialias {
            self.draw(skin, scene, out);
            return;
        }
        let mut big = self.big.take().unwrap_or_else(|| RgbaFrame::transparent(0, 0));
        let s2 = Scene { width: scene.width * 2, height: scene.height * 2, antialias: false, ..*scene };
        self.draw(skin, &s2, &mut big);
        downsample_2x(&big, out, scene.width, scene.height);
        self.big = Some(big);
    }

    fn draw(&mut self, skin: &SkinTexture, scene: &Scene, out: &mut RgbaFrame) {
        out.width = scene.width;
        out.height = scene.height;
        let half_fov = (FOV_DEG.to_radians() / 2.0).tan();
        let zoom = (BASE_ZOOM * scene.zoom).max(0.05);
        let distance = (4.5 + 16.5 / half_fov / zoom).clamp(24.0, 256.0);
        let focal = scene.height as f32 * 0.5 / half_fov;
        let pose = &scene.pose;
        let view = Xf::rotate(M3::rot_x(pose.pitch_deg.to_radians()).mul(M3::rot_y(pose.yaw_deg.to_radians())));
        model::build(scene.model, pose, &view, scene.overlay, &mut self.quads);
        let mut r = Raster {
            frame: out,
            depth: &mut self.depth,
            proj: Projection { width: scene.width, height: scene.height, focal, distance },
            light: v3(LIGHT.0, LIGHT.1, LIGHT.2).normalized(),
            ambient: if scene.shading { AMBIENT } else { 1.0 },
            diffuse: if scene.shading { DIFFUSE } else { 0.0 },
        };
        r.clear();
        if scene.width == 0 || scene.height == 0 {
            return;
        }
        let tex = Tex { rgba: &skin.rgba, width: skin.width, scale: (skin.width / 64).max(1) };
        r.draw_all(&self.quads, &tex);
    }
}

/// Box-filters a 2x frame down with premultiplied alpha (straight in / out).
fn downsample_2x(big: &RgbaFrame, out: &mut RgbaFrame, w: u32, h: u32) {
    out.width = w;
    out.height = h;
    out.pixels.clear();
    out.pixels.resize((w * h * 4) as usize, 0);
    let bw = big.width as usize;
    for y in 0..h as usize {
        for x in 0..w as usize {
            let i0 = (2 * y * bw + 2 * x) * 4;
            let i1 = i0 + bw * 4;
            let q = [
                &big.pixels[i0..i0 + 4],
                &big.pixels[i0 + 4..i0 + 8],
                &big.pixels[i1..i1 + 4],
                &big.pixels[i1 + 4..i1 + 8],
            ];
            let o = (y * w as usize + x) * 4;
            let px = &mut out.pixels[o..o + 4];
            if q.iter().all(|p| p[3] == 255) {
                for k in 0..3 {
                    px[k] = ((q[0][k] as u32 + q[1][k] as u32 + q[2][k] as u32 + q[3][k] as u32 + 2) >> 2) as u8;
                }
                px[3] = 255;
                continue;
            }
            let mut acc = [0u32; 4];
            for p in q {
                let a = p[3] as u32;
                acc[0] += p[0] as u32 * a;
                acc[1] += p[1] as u32 * a;
                acc[2] += p[2] as u32 * a;
                acc[3] += a;
            }
            if let (Some(r), Some(g), Some(b)) =
                (acc[0].checked_div(acc[3]), acc[1].checked_div(acc[3]), acc[2].checked_div(acc[3]))
            {
                px.copy_from_slice(&[r as u8, g as u8, b as u8, ((acc[3] + 2) / 4) as u8]);
            }
        }
    }
}

/// One-shot convenience: renders `skin` (with its arm `model`) in `pose`.
pub fn render(skin: &SkinTexture, model: ArmModel, pose: &Pose, width: u32, height: u32) -> RgbaFrame {
    Rasterizer::new().render(skin, &Scene::new(width, height, model, *pose))
}

/// A skin renderer. Implementations must be cheap to call repeatedly.
pub trait SkinRenderer: Send + Sync {
    fn render(&self, skin: &SkinTexture, params: &RenderParams) -> RgbaFrame;
}

/// The CPU 3D renderer behind [`default_renderer`]; reuses its buffers between frames.
#[derive(Default)]
pub struct SoftwareRenderer {
    scratch: Mutex<Rasterizer>,
    /// Supersample every frame (see [`Scene::antialias`]).
    pub antialias: bool,
}

impl SoftwareRenderer {
    pub fn new(antialias: bool) -> SoftwareRenderer {
        SoftwareRenderer { scratch: Mutex::new(Rasterizer::new()), antialias }
    }
}

impl SkinRenderer for SoftwareRenderer {
    fn render(&self, skin: &SkinTexture, params: &RenderParams) -> RgbaFrame {
        let scene = Scene { antialias: self.antialias, ..Scene::from(params) };
        match self.scratch.try_lock() {
            Ok(mut r) => r.render(skin, &scene),
            Err(_) => Rasterizer::new().render(skin, &scene),
        }
    }
}

/// The renderer the app uses.
pub fn default_renderer() -> Box<dyn SkinRenderer> {
    Box::new(SoftwareRenderer::new(false))
}

#[cfg(test)]
mod tests;
