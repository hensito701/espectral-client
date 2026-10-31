//! The player model: six body parts, each an inner box + an inflated outer
//! (second layer) box, with the exact geometry, pivots and UV layout of
//! skinview3d 3.4 `SkinObject` / `PlayerObject` (which match Minecraft's).
//!
//! Model space is skinview3d's: y up, the player faces +z, his left side is
//! +x, the figure spans y = -16..16 (feet .. top of the head), centred on 0.

use crate::math::{v3, Xf, M3, V3};
use crate::{ArmModel, Pose};

/// Which skin layer a face belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    /// Base layer: opaque, back faces culled.
    Inner,
    /// Second layer: alpha-tested / blended, double sided.
    Outer,
}

/// One textured quad in view space. Corners are TL, TR, BL, BR as seen from
/// outside the box, matching the texture rect orientation.
#[derive(Debug, Clone, Copy)]
pub struct Quad {
    pub pts: [V3; 4],
    /// Outward normal (view space).
    pub normal: V3,
    /// Texture rect in 64-space texels: u0, v0, w, h.
    pub rect: (u32, u32, u32, u32),
    pub layer: Layer,
}

/// A box with its 64-space UV origin and its *texture* dimensions (the outer
/// boxes are bigger than the texels they map).
struct BoxDef {
    size: V3,
    uv: (u32, u32),
    tex: (u32, u32, u32),
}

/// Corners (TL, TR, BL, BR), outward normal, 64-space texture rect.
type Face = ([V3; 4], V3, (u32, u32, u32, u32));

/// Emits the 6 faces of `b` (centred at the origin of `xf`).
fn push_box(out: &mut Vec<Quad>, xf: &Xf, b: &BoxDef, layer: Layer) {
    let (hx, hy, hz) = (b.size.x / 2.0, b.size.y / 2.0, b.size.z / 2.0);
    let (u, v) = b.uv;
    let (w, h, d) = b.tex;
    // (TL, TR, BL, BR), outward normal, texture rect. See setUVs() in
    // skinview3d/libs/model.js and three.js BoxGeometry face order.
    let faces: [Face; 6] = [
        // front (+z)
        ([v3(-hx, hy, hz), v3(hx, hy, hz), v3(-hx, -hy, hz), v3(hx, -hy, hz)], v3(0.0, 0.0, 1.0), (u + d, v + d, w, h)),
        // back (-z)
        (
            [v3(hx, hy, -hz), v3(-hx, hy, -hz), v3(hx, -hy, -hz), v3(-hx, -hy, -hz)],
            v3(0.0, 0.0, -1.0),
            (u + 2 * d + w, v + d, w, h),
        ),
        // player's right side (-x)
        (
            [v3(-hx, hy, -hz), v3(-hx, hy, hz), v3(-hx, -hy, -hz), v3(-hx, -hy, hz)],
            v3(-1.0, 0.0, 0.0),
            (u, v + d, d, h),
        ),
        // player's left side (+x)
        (
            [v3(hx, hy, hz), v3(hx, hy, -hz), v3(hx, -hy, hz), v3(hx, -hy, -hz)],
            v3(1.0, 0.0, 0.0),
            (u + d + w, v + d, d, h),
        ),
        // top (+y)
        ([v3(-hx, hy, -hz), v3(hx, hy, -hz), v3(-hx, hy, hz), v3(hx, hy, hz)], v3(0.0, 1.0, 0.0), (u + d, v, w, d)),
        // bottom (-y)
        (
            [v3(-hx, -hy, -hz), v3(hx, -hy, -hz), v3(-hx, -hy, hz), v3(hx, -hy, hz)],
            v3(0.0, -1.0, 0.0),
            (u + d + w, v, w, d),
        ),
    ];
    for (pts, n, rect) in faces {
        out.push(Quad { pts: pts.map(|p| xf.point(p)), normal: xf.dir(n), rect, layer });
    }
}

struct Part {
    /// Joint position in model space.
    pivot: V3,
    /// Joint rotation (Euler XYZ, radians).
    rot: (f32, f32, f32),
    /// Box centre relative to the joint.
    offset: V3,
    inner: BoxDef,
    outer: BoxDef,
}

/// Builds the six parts for `model` in `pose`.
fn parts(model: ArmModel, pose: &Pose) -> [Part; 6] {
    let slim = model == ArmModel::Slim;
    let aw = if slim { 3.0 } else { 4.0 };
    let awt = if slim { 3 } else { 4 };
    let arm_dx = if slim { 0.5 } else { 1.0 };
    // skinview3d IdleAnimation: arms rest slightly open and sway around it.
    let base_z = std::f32::consts::PI * 0.02 * pose.arm_rest;
    let sway = pose.idle_phase.cos() * 0.03 * pose.arm_rest;
    // WalkingAnimation: limbs swing on x, the head bobs.
    let swing = pose.walk_phase.sin() * pose.arm_swing;
    let head_y = (pose.walk_phase / 4.0).sin() * 0.2 * pose.head_bob;
    let head_x = (pose.walk_phase / 5.0).sin() * 0.1 * pose.head_bob;
    let ob = |sx: f32, sy: f32, sz: f32, uv: (u32, u32), tex: (u32, u32, u32)| BoxDef { size: v3(sx, sy, sz), uv, tex };
    [
        Part {
            pivot: v3(0.0, 8.0, 0.0),
            rot: (head_x, head_y, 0.0),
            offset: v3(0.0, 4.0, 0.0),
            inner: ob(8.0, 8.0, 8.0, (0, 0), (8, 8, 8)),
            outer: ob(9.0, 9.0, 9.0, (32, 0), (8, 8, 8)),
        },
        Part {
            pivot: v3(0.0, 2.0, 0.0),
            rot: (0.0, 0.0, 0.0),
            offset: v3(0.0, 0.0, 0.0),
            inner: ob(8.0, 12.0, 4.0, (16, 16), (8, 12, 4)),
            outer: ob(8.5, 12.5, 4.5, (16, 32), (8, 12, 4)),
        },
        // right arm (-x)
        Part {
            pivot: v3(-5.0, 6.0, 0.0),
            rot: (swing, 0.0, -sway - base_z),
            offset: v3(-arm_dx, -4.0, 0.0),
            inner: ob(aw, 12.0, 4.0, (40, 16), (awt, 12, 4)),
            outer: ob(aw + 0.5, 12.5, 4.5, (40, 32), (awt, 12, 4)),
        },
        // left arm (+x)
        Part {
            pivot: v3(5.0, 6.0, 0.0),
            rot: (-swing, 0.0, sway + base_z),
            offset: v3(arm_dx, -4.0, 0.0),
            inner: ob(aw, 12.0, 4.0, (32, 48), (awt, 12, 4)),
            outer: ob(aw + 0.5, 12.5, 4.5, (48, 48), (awt, 12, 4)),
        },
        // right leg
        Part {
            pivot: v3(-1.9, -4.0, -0.1),
            rot: (-swing, 0.0, 0.0),
            offset: v3(0.0, -6.0, 0.0),
            inner: ob(4.0, 12.0, 4.0, (0, 16), (4, 12, 4)),
            outer: ob(4.5, 12.5, 4.5, (0, 32), (4, 12, 4)),
        },
        // left leg
        Part {
            pivot: v3(1.9, -4.0, -0.1),
            rot: (swing, 0.0, 0.0),
            offset: v3(0.0, -6.0, 0.0),
            inner: ob(4.0, 12.0, 4.0, (16, 48), (4, 12, 4)),
            outer: ob(4.5, 12.5, 4.5, (0, 48), (4, 12, 4)),
        },
    ]
}

/// All quads of the player, transformed by `view` (model -> view space).
pub fn build(model: ArmModel, pose: &Pose, view: &Xf, overlay: bool, out: &mut Vec<Quad>) {
    out.clear();
    for p in parts(model, pose) {
        let joint = view
            .then_inner(Xf::translate(p.pivot))
            .then_inner(Xf::rotate(M3::euler_xyz(p.rot.0, p.rot.1, p.rot.2)))
            .then_inner(Xf::translate(p.offset));
        push_box(out, &joint, &p.inner, Layer::Inner);
        if overlay {
            push_box(out, &joint, &p.outer, Layer::Outer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn figure_spans_minus16_to_16() {
        let mut q = Vec::new();
        build(ArmModel::Classic, &Pose::default(), &Xf::IDENTITY, false, &mut q);
        assert_eq!(q.len(), 36);
        let ys: Vec<f32> = q.iter().flat_map(|q| q.pts.iter().map(|p| p.y)).collect();
        let min = ys.iter().cloned().fold(f32::MAX, f32::min);
        let max = ys.iter().cloned().fold(f32::MIN, f32::max);
        assert!((min + 16.0).abs() < 1e-4 && (max - 16.0).abs() < 1e-4, "{min}..{max}");
    }

    #[test]
    fn faces_are_wound_outward() {
        let mut q = Vec::new();
        build(ArmModel::Slim, &Pose::default(), &Xf::IDENTITY, true, &mut q);
        assert_eq!(q.len(), 72);
        for f in &q {
            // Seen from outside, TL -> TR -> BL runs clockwise, i.e. (TR - TL) x (BL - TL)
            // points inward; the bottom faces are mirrored (skinview3d / Minecraft UVs).
            let a = f.pts[1] - f.pts[0];
            let b = f.pts[2] - f.pts[0];
            let cross = v3(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x);
            let mirrored = f.normal.y < -0.5;
            assert_eq!(cross.dot(f.normal) > 0.0, mirrored, "{f:?}");
        }
    }
}
