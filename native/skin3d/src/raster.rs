//! Z-buffered, perspective-correct, nearest-neighbour textured quad rasterizer.
//!
//! - Vertices are snapped to 1/256 px and edges are evaluated with exact
//!   integer edge functions + the top-left fill rule, so the two triangles of
//!   a quad (and adjacent quads) never leave cracks or blend a pixel twice.
//! - Attributes (u/w, v/w, 1/w) are affine in screen space: one plane
//!   equation per quad, stepped incrementally per pixel; one division per
//!   covered pixel recovers (u, v).
//! - Depth = 1/w (bigger = nearer), cleared to 0.

use crate::math::V3;
use crate::model::{Layer, Quad};
use crate::RgbaFrame;

const SUB: i64 = 256; // subpixel steps per pixel
const HALF: i64 = SUB / 2;

/// Screen-space vertex.
#[derive(Clone, Copy, Debug)]
struct SV {
    /// Snapped fixed-point position.
    fx: i64,
    fy: i64,
    /// Same position in pixels (float, from the snapped value).
    x: f32,
    y: f32,
    inv_w: f32,
    u: f32,
    v: f32,
}

/// Perspective camera: view space looks down -z from the origin.
#[derive(Clone, Copy, Debug)]
pub struct Projection {
    pub width: u32,
    pub height: u32,
    /// Focal length in pixels.
    pub focal: f32,
    /// Distance from the camera to the model origin.
    pub distance: f32,
}

impl Projection {
    fn project(&self, p: V3, u: f32, v: f32) -> SV {
        let w = (self.distance - p.z).max(0.01);
        let inv_w = 1.0 / w;
        let x = self.width as f32 * 0.5 + p.x * self.focal * inv_w;
        let y = self.height as f32 * 0.5 - p.y * self.focal * inv_w;
        let fx = (x * SUB as f32).round() as i64;
        let fy = (y * SUB as f32).round() as i64;
        SV { fx, fy, x: fx as f32 / SUB as f32, y: fy as f32 / SUB as f32, inv_w, u: u * inv_w, v: v * inv_w }
    }
}

/// How a quad writes pixels.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Opaque,
    Blend,
}

/// Texture access for the rasterizer: square, 1.8 layout, `scale` texels per 64-space unit.
pub struct Tex<'a> {
    pub rgba: &'a [u8],
    pub width: u32,
    pub scale: u32,
}

impl Tex<'_> {
    /// True when every texel of the 64-space rect is fully transparent
    /// (unused second-layer faces are skipped instead of rasterized).
    fn face_is_empty(&self, (u, v, w, h): (u32, u32, u32, u32)) -> bool {
        let s = self.scale;
        (v * s..(v + h) * s).all(|y| {
            let row = ((y * self.width + u * s) * 4) as usize;
            self.rgba[row..row + (w * s * 4) as usize].chunks_exact(4).all(|p| p[3] == 0)
        })
    }
}

pub struct Raster<'a> {
    pub frame: &'a mut RgbaFrame,
    pub depth: &'a mut Vec<f32>,
    pub proj: Projection,
    /// Unit light direction in view space (towards the light).
    pub light: V3,
    pub ambient: f32,
    pub diffuse: f32,
}

impl Raster<'_> {
    pub fn clear(&mut self) {
        let n = (self.proj.width * self.proj.height) as usize;
        self.frame.pixels.clear();
        self.frame.pixels.resize(n * 4, 0);
        self.depth.clear();
        self.depth.resize(n, 0.0);
    }

    /// Draws every quad: base layer (culled), then second-layer back faces,
    /// then second-layer front faces (so translucent texels blend over what
    /// is behind them).
    pub fn draw_all(&mut self, quads: &[Quad], tex: &Tex) {
        for pass in 0..3 {
            for q in quads {
                let front = self.facing_camera(q);
                let draw = match (pass, q.layer) {
                    (0, Layer::Inner) => front,
                    (1, Layer::Outer) => !front,
                    (2, Layer::Outer) => front,
                    _ => false,
                };
                if draw && !(q.layer == Layer::Outer && tex.face_is_empty(q.rect)) {
                    self.draw_quad(q, tex, front);
                }
            }
        }
    }

    /// True when the outward side of `q` faces the camera.
    fn facing_camera(&self, q: &Quad) -> bool {
        // Camera at (0, 0, distance) in model-view space (view looks down -z).
        let to_cam = V3 { x: -q.pts[0].x, y: -q.pts[0].y, z: self.proj.distance - q.pts[0].z };
        q.normal.dot(to_cam) > 0.0
    }

    fn shade(&self, n: V3) -> u32 {
        let lit = self.ambient + self.diffuse * n.dot(self.light).max(0.0);
        (lit.clamp(0.0, 1.0) * 256.0) as u32
    }

    fn draw_quad(&mut self, q: &Quad, tex: &Tex, front: bool) {
        let s = tex.scale;
        let (ru, rv, rw, rh) = q.rect;
        let (u0, v0, u1, v1) = ((ru * s) as f32, (rv * s) as f32, ((ru + rw) * s) as f32, ((rv + rh) * s) as f32);
        let clamp = (ru * s, rv * s, (ru + rw) * s - 1, (rv + rh) * s - 1);
        let p = &self.proj;
        let tl = p.project(q.pts[0], u0, v0);
        let tr = p.project(q.pts[1], u1, v0);
        let bl = p.project(q.pts[2], u0, v1);
        let br = p.project(q.pts[3], u1, v1);
        let normal = if front { q.normal } else { -q.normal };
        let shade = self.shade(normal);
        let mode = if q.layer == Layer::Inner { Mode::Opaque } else { Mode::Blend };
        // Plane equations of u/w, v/w, 1/w over the quad (affine in screen space).
        let Some(planes) = Planes::new(&tl, &tr, &bl) else { return };
        let ctx = TriCtx { planes, clamp, shade, mode };
        // Visibility was decided from the normal; the corner order only says
        // how the texture lies on the face (the bottom faces are mirrored, as
        // in skinview3d), so pick the winding that is positive on screen.
        if orient(&tl, &tr, bl.fx, bl.fy) > 0 {
            self.tri(&tl, &tr, &bl, &ctx, tex);
            self.tri(&tr, &br, &bl, &ctx, tex);
        } else {
            self.tri(&tl, &bl, &tr, &ctx, tex);
            self.tri(&tr, &bl, &br, &ctx, tex);
        }
    }

    fn tri(&mut self, a: &SV, b: &SV, c: &SV, ctx: &TriCtx, tex: &Tex) {
        let area = orient(a, b, c.fx, c.fy);
        if area <= 0 {
            return; // degenerate (edge-on) or wrong winding
        }
        let (w, h) = (self.proj.width as i64, self.proj.height as i64);
        let min_x = (a.fx.min(b.fx).min(c.fx) / SUB).max(0);
        let max_x = ((a.fx.max(b.fx).max(c.fx) + SUB - 1) / SUB).min(w - 1);
        let min_y = (a.fy.min(b.fy).min(c.fy) / SUB).max(0);
        let max_y = ((a.fy.max(b.fy).max(c.fy) + SUB - 1) / SUB).min(h - 1);
        if min_x > max_x || min_y > max_y {
            return;
        }
        let edges = [Edge::new(b, c), Edge::new(c, a), Edge::new(a, b)];
        let px0 = min_x * SUB + HALF;
        let mut py = min_y * SUB + HALF;
        let fw = self.proj.width as usize;
        let tw = tex.width as usize;
        let pl = &ctx.planes;
        let (cu0, cv0, cu1, cv1) = (ctx.clamp.0 as i32, ctx.clamp.1 as i32, ctx.clamp.2 as i32, ctx.clamp.3 as i32);
        let sh = ctx.shade;
        for y in min_y..=max_y {
            // Covered span of this row: solve edge(x) = w + k * dx >= 0 for every edge.
            let (mut k0, mut k1) = (0i64, max_x - min_x);
            for e in &edges {
                let w = e.at(px0, py);
                if e.dx == 0 {
                    if w < 0 {
                        k1 = -1;
                    }
                } else if e.dx > 0 {
                    if w < 0 {
                        k0 = k0.max((-w + e.dx - 1) / e.dx);
                    }
                } else {
                    k1 = k1.min(if w < 0 { -1 } else { w / -e.dx });
                }
            }
            py += SUB;
            if k0 > k1 {
                continue;
            }
            let x0 = min_x + k0;
            let yc = y as f32 + 0.5;
            let xc = x0 as f32 + 0.5;
            let mut iw = pl.iw.at(xc, yc);
            let mut uw = pl.u.at(xc, yc);
            let mut vw = pl.v.at(xc, yc);
            let row = y as usize * fw;
            let depth = &mut self.depth[row + x0 as usize..=row + (min_x + k1) as usize];
            let pixels = &mut self.frame.pixels[(row + x0 as usize) * 4..(row + (min_x + k1) as usize + 1) * 4];
            for (d, px) in depth.iter_mut().zip(pixels.chunks_exact_mut(4)) {
                if iw > *d {
                    let inv = 1.0 / iw;
                    let tu = ((uw * inv) as i32).clamp(cu0, cu1) as usize;
                    let tv = ((vw * inv) as i32).clamp(cv0, cv1) as usize;
                    let ti = (tv * tw + tu) * 4;
                    let t: [u8; 4] = tex.rgba[ti..ti + 4].try_into().unwrap();
                    let shaded = [
                        ((t[0] as u32 * sh) >> 8) as u8,
                        ((t[1] as u32 * sh) >> 8) as u8,
                        ((t[2] as u32 * sh) >> 8) as u8,
                        255,
                    ];
                    match ctx.mode {
                        // Base layer ignores alpha (three.js opaque material):
                        // transparent texels come out black.
                        Mode::Opaque => {
                            px.copy_from_slice(if t[3] == 0 { &[0, 0, 0, 255] } else { &shaded });
                            *d = iw;
                        }
                        Mode::Blend => {
                            if t[3] == 255 {
                                px.copy_from_slice(&shaded);
                                *d = iw;
                            } else if t[3] > 0 {
                                blend_over(px, [shaded[0] as u32, shaded[1] as u32, shaded[2] as u32], t[3] as u32);
                                *d = iw;
                            }
                        }
                    }
                }
                iw += pl.iw.a;
                uw += pl.u.a;
                vw += pl.v.a;
            }
        }
    }
}

/// Straight-alpha "over" of `src` (alpha `a`, 1..254) onto `dst`.
fn blend_over(dst: &mut [u8], src: [u32; 3], a: u32) {
    let da = dst[3] as u32;
    let out_a = a * 255 + da * (255 - a); // scaled by 255
    if out_a == 0 {
        return;
    }
    for k in 0..3 {
        let num = src[k] * a * 255 + dst[k] as u32 * da * (255 - a);
        dst[k] = (num / out_a) as u8;
    }
    dst[3] = (out_a / 255) as u8;
}

/// 2x signed area of (a, b, p) in fixed point; > 0 for clockwise-on-screen (y down).
fn orient(a: &SV, b: &SV, px: i64, py: i64) -> i64 {
    (b.fx - a.fx) * (py - a.fy) - (b.fy - a.fy) * (px - a.fx)
}

struct Edge {
    a_fx: i64,
    a_fy: i64,
    ex: i64,
    ey: i64,
    /// Value step per pixel in x.
    dx: i64,
    /// 0 for top-left edges (pixels exactly on them are drawn), -1 otherwise.
    bias: i64,
}

impl Edge {
    fn new(a: &SV, b: &SV) -> Edge {
        let ex = b.fx - a.fx;
        let ey = b.fy - a.fy;
        let top_left = (ey == 0 && ex > 0) || ey < 0;
        Edge { a_fx: a.fx, a_fy: a.fy, ex, ey, dx: -ey * SUB, bias: if top_left { 0 } else { -1 } }
    }
    fn at(&self, px: i64, py: i64) -> i64 {
        self.ex * (py - self.a_fy) - self.ey * (px - self.a_fx) + self.bias
    }
}

#[derive(Clone, Copy)]
struct Plane {
    c: f32,
    a: f32,
    b: f32,
}

impl Plane {
    #[inline]
    fn at(&self, x: f32, y: f32) -> f32 {
        self.c + self.a * x + self.b * y
    }
}

struct Planes {
    u: Plane,
    v: Plane,
    iw: Plane,
}

impl Planes {
    fn new(p0: &SV, p1: &SV, p2: &SV) -> Option<Planes> {
        let (x1, y1) = (p1.x - p0.x, p1.y - p0.y);
        let (x2, y2) = (p2.x - p0.x, p2.y - p0.y);
        let det = x1 * y2 - x2 * y1;
        if det.abs() < 1e-6 {
            return None;
        }
        let inv = 1.0 / det;
        let plane = |f0: f32, f1: f32, f2: f32| {
            let (d1, d2) = (f1 - f0, f2 - f0);
            let a = (d1 * y2 - d2 * y1) * inv;
            let b = (d2 * x1 - d1 * x2) * inv;
            Plane { c: f0 - a * p0.x - b * p0.y, a, b }
        };
        Some(Planes { u: plane(p0.u, p1.u, p2.u), v: plane(p0.v, p1.v, p2.v), iw: plane(p0.inv_w, p1.inv_w, p2.inv_w) })
    }
}

struct TriCtx {
    planes: Planes,
    /// Inclusive texel bounds of the face (no bleeding into neighbours).
    clamp: (u32, u32, u32, u32),
    shade: u32,
    mode: Mode,
}
