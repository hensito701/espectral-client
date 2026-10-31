//! espectral-skin3d — native Minecraft skin viewer (replaces skinview3d / three.js).
//!
//! OWNER: the skin-renderer agent (see `native/app/UI-PORTING.md`). The app
//! only depends on the API below; everything behind it is free to change.
//!
//! Contract: a CPU rasterizer that turns a Minecraft skin texture (64x64 or
//! legacy 64x32 RGBA) into an RGBA frame of the requested size for a given
//! camera (yaw / pitch / zoom) and arm model. The UI calls it from a Slint
//! pure callback on the UI thread whenever an input changes (drag-to-rotate,
//! resize, new texture), so a frame must render in a few milliseconds at
//! ~300x400 and nothing may run while the inputs are unchanged (idle = 0% CPU).
//!
//! The current [`StubRenderer`] only draws the flat front of the head and
//! body so pages can be laid out before the real renderer lands.

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

/// Decoded skin texture, straight RGBA8 (not premultiplied), row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkinTexture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl SkinTexture {
    /// `None` unless the buffer matches the size and the texture is 64x64 or 64x32
    /// (or an HD multiple of those).
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Option<Self> {
        let ok_shape = width >= 64 && width % 64 == 0 && (height == width || height * 2 == width);
        if !ok_shape || rgba.len() != (width as usize) * (height as usize) * 4 {
            return None;
        }
        Some(SkinTexture { width, height, rgba })
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

/// Camera + pose for one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderParams {
    /// Output size in physical pixels.
    pub width: u32,
    pub height: u32,
    /// Rotation around the vertical axis, degrees (0 = facing the camera).
    pub yaw_deg: f32,
    /// Rotation around the horizontal axis, degrees (positive = look from above).
    pub pitch_deg: f32,
    /// 1.0 = the whole player fits the frame.
    pub zoom: f32,
    pub model: ArmModel,
    /// Animation clock in seconds (idle sway / walk); 0 = static pose.
    pub time_s: f32,
    /// Draw the second (overlay) layer.
    pub overlay: bool,
}

impl Default for RenderParams {
    fn default() -> Self {
        RenderParams {
            width: 280,
            height: 340,
            yaw_deg: -25.0,
            pitch_deg: 8.0,
            zoom: 1.0,
            model: ArmModel::Classic,
            time_s: 0.0,
            overlay: true,
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

    fn put(&mut self, x: u32, y: u32, c: [u8; 4]) {
        if x < self.width && y < self.height && c[3] > 0 {
            let i = ((y * self.width + x) * 4) as usize;
            self.pixels[i..i + 4].copy_from_slice(&c);
        }
    }
}

/// A skin renderer. Implementations must be cheap to call repeatedly.
pub trait SkinRenderer: Send + Sync {
    fn render(&self, skin: &SkinTexture, params: &RenderParams) -> RgbaFrame;
}

/// Placeholder: flat front view (head + body + arms + legs), no 3D.
#[derive(Debug, Default, Clone, Copy)]
pub struct StubRenderer;

impl SkinRenderer for StubRenderer {
    fn render(&self, skin: &SkinTexture, p: &RenderParams) -> RgbaFrame {
        let mut out = RgbaFrame::transparent(p.width, p.height);
        // Front faces in skin space: (u, v, w, h) and their place on a 16x32 grid.
        let arm_w = if p.model == ArmModel::Slim { 3 } else { 4 };
        let legacy = skin.height * 2 == skin.width;
        let parts: [(u32, u32, u32, u32, i32, i32); 6] = [
            (8, 8, 8, 8, 4, 0),                                                             // head
            (20, 20, 8, 12, 4, 8),                                                          // body
            (44, 20, arm_w, 12, 4 - arm_w as i32, 8),                                       // right arm
            (if legacy { 44 } else { 36 }, if legacy { 20 } else { 52 }, arm_w, 12, 12, 8), // left arm
            (4, 20, 4, 12, 4, 20),                                                          // right leg
            (if legacy { 4 } else { 20 }, if legacy { 20 } else { 52 }, 4, 12, 8, 20),      // left leg
        ];
        let scale = ((p.height as f32 * 0.9 * p.zoom) / 32.0).min(p.width as f32 * 0.9 / 16.0).max(1.0);
        let ox = (p.width as f32 - 16.0 * scale) / 2.0;
        let oy = (p.height as f32 - 32.0 * scale) / 2.0;
        for &(u, v, w, h, gx, gy) in &parts {
            for ty in 0..h {
                for tx in 0..w {
                    let c = skin.texel(u + tx, v + ty);
                    let x0 = (ox + (gx as f32 + tx as f32) * scale) as u32;
                    let y0 = (oy + (gy as f32 + ty as f32) * scale) as u32;
                    let s = scale.ceil() as u32;
                    for dy in 0..s {
                        for dx in 0..s {
                            out.put(x0 + dx, y0 + dy, c);
                        }
                    }
                }
            }
        }
        out
    }
}

/// The renderer the app uses. The skin-renderer agent swaps this for the real one.
pub fn default_renderer() -> Box<dyn SkinRenderer> {
    Box::new(StubRenderer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_renders_requested_size() {
        let skin = SkinTexture::new(64, 64, vec![255; 64 * 64 * 4]).unwrap();
        let f = default_renderer().render(&skin, &RenderParams { width: 100, height: 200, ..Default::default() });
        assert_eq!(f.pixels.len(), 100 * 200 * 4);
        assert!(f.pixels.contains(&255));
    }

    #[test]
    fn rejects_bad_shapes() {
        assert!(SkinTexture::new(64, 48, vec![0; 64 * 48 * 4]).is_none());
        assert!(SkinTexture::new(64, 32, vec![0; 64 * 32 * 4]).is_some());
    }
}
