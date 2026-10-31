//! Semantic renderer tests (UV mapping, orientation, layers, models). Golden
//! image comparisons live in `tests/golden.rs`.

use super::*;
use crate::fixtures::{face_color, face_id_rgba, steve, steve_legacy_rgba, FACES};

const W: u32 = 200;
const H: u32 = 300;

fn flat(pose: Pose, model: ArmModel) -> Scene {
    Scene { shading: false, ..Scene::new(W, H, model, pose) }
}

fn face_id() -> SkinTexture {
    SkinTexture::new(64, 64, face_id_rgba()).unwrap()
}

fn render_flat(skin: &SkinTexture, pose: Pose) -> RgbaFrame {
    Rasterizer::new().render(skin, &flat(pose, ArmModel::Classic))
}

/// Screen position of a model-space point for an unrotated pose (same camera
/// maths as `Rasterizer::draw`).
fn screen(x: f32, y: f32, z: f32) -> (u32, u32) {
    let half = (FOV_DEG.to_radians() / 2.0).tan();
    let d = 4.5 + 16.5 / half / BASE_ZOOM;
    let f = H as f32 * 0.5 / half;
    let w = d - z;
    ((W as f32 * 0.5 + x * f / w) as u32, (H as f32 * 0.5 - y * f / w) as u32)
}

/// (part, face) whose id colour is `c`.
fn which(c: [u8; 4]) -> Option<(usize, &'static str)> {
    (0..6).flat_map(|p| (0..6).map(move |f| (p, f))).find(|&(p, f)| face_color(p, f) == c).map(|(p, f)| (p, FACES[f]))
}

fn yaw(deg: f32) -> Pose {
    Pose { yaw_deg: deg, arm_rest: 0.0, ..Pose::default() }
}

#[test]
fn frame_has_requested_size_and_transparent_background() {
    let f = render(&steve(), ArmModel::Classic, &Pose::default(), 100, 160);
    assert_eq!((f.width, f.height, f.pixels.len()), (100, 160, 100 * 160 * 4));
    assert_eq!(f.pixel(0, 0)[3], 0);
    assert_eq!(f.pixel(99, 159)[3], 0);
    assert!(f.pixels.chunks_exact(4).filter(|p| p[3] == 255).count() > 1000);
}

#[test]
fn front_view_shows_front_faces() {
    let f = render_flat(&face_id(), yaw(0.0));
    let (x, y) = screen(0.0, 12.0, 4.0);
    assert_eq!(which(f.pixel(x, y)), Some((0, "front")), "head");
    let (x, y) = screen(0.0, 2.0, 2.0);
    assert_eq!(which(f.pixel(x, y)), Some((1, "front")), "body");
    // the player's right arm / leg are on the viewer's left
    let (x, y) = screen(-6.0, 2.0, 2.0);
    assert_eq!(which(f.pixel(x, y)), Some((2, "front")), "right arm");
    let (x, y) = screen(6.0, 2.0, 2.0);
    assert_eq!(which(f.pixel(x, y)), Some((3, "front")), "left arm");
    let (x, y) = screen(-2.0, -10.0, 2.0);
    assert_eq!(which(f.pixel(x, y)), Some((4, "front")), "right leg");
    let (x, y) = screen(2.0, -10.0, 2.0);
    assert_eq!(which(f.pixel(x, y)), Some((5, "front")), "left leg");
}

#[test]
fn turning_shows_the_matching_side() {
    let (x, y) = screen(0.0, 12.0, 4.0);
    let back = render_flat(&face_id(), yaw(180.0));
    assert_eq!(which(back.pixel(x, y)), Some((0, "back")));
    // +90: his front turns to the viewer's right, so his right side (-x) faces us
    let r = render_flat(&face_id(), yaw(90.0));
    assert_eq!(which(r.pixel(x, y)), Some((0, "right")));
    let l = render_flat(&face_id(), yaw(-90.0));
    assert_eq!(which(l.pixel(x, y)), Some((0, "left")));
}

#[test]
fn looking_from_above_shows_the_top_of_the_head() {
    let f = render_flat(&face_id(), Pose { pitch_deg: 85.0, ..yaw(0.0) });
    // the head top is now (almost) facing the camera, centred near the screen centre
    let c = f.pixel(W / 2, H / 2 - 2);
    assert_eq!(which(c), Some((0, "top")), "{c:?}");
}

#[test]
fn texture_orientation_inside_a_face() {
    // Mark texel (8, 8) = top-left of the head front, and (15, 15) = bottom-right.
    let mut rgba = face_id_rgba();
    let mut mark = |x: u32, y: u32, c: [u8; 4]| {
        let i = ((y * 64 + x) * 4) as usize;
        rgba[i..i + 4].copy_from_slice(&c);
    };
    mark(8, 8, [255, 0, 255, 255]);
    mark(15, 15, [0, 255, 255, 255]);
    // head top: texel (8, 0) is its back-left corner (-x, -z), (15, 7) front-right
    mark(8, 0, [255, 255, 0, 255]);
    let skin = SkinTexture::new(64, 64, rgba).unwrap();
    let f = render_flat(&skin, yaw(0.0));
    let (x, y) = screen(-3.5, 15.5, 4.0);
    assert_eq!(f.pixel(x, y), [255, 0, 255, 255], "TL texel at the viewer's top-left");
    let (x, y) = screen(3.5, 8.5, 4.0);
    assert_eq!(f.pixel(x, y), [0, 255, 255, 255], "BR texel at the viewer's bottom-right");
    let top = render_flat(&skin, Pose { pitch_deg: 89.0, ..yaw(0.0) });
    // seen from above, -z (the back) is at the top of the screen and -x on the left
    let mut found = None;
    for yy in 0..H {
        for xx in 0..W {
            if top.pixel(xx, yy) == [255, 255, 0, 255] {
                found.get_or_insert((xx, yy));
            }
        }
    }
    let (fx, fy) = found.expect("hat-top marker visible");
    assert!(fx < W / 2 && fy < H / 2, "marker at {fx},{fy}");
}

#[test]
fn faces_are_watertight() {
    // A big front view: every pixel inside the head front must be covered.
    let f = Rasterizer::new()
        .render(&face_id(), &Scene { shading: false, ..Scene::new(400, 600, ArmModel::Classic, yaw(0.0)) });
    let half = (FOV_DEG.to_radians() / 2.0).tan();
    let d = 4.5 + 16.5 / half / BASE_ZOOM;
    let fo = 300.0 / half;
    let to = |x: f32, y: f32| ((200.0 + x * fo / (d - 4.0)) as u32, (300.0 - y * fo / (d - 4.0)) as u32);
    let (x0, y0) = to(-3.9, 15.9);
    let (x1, y1) = to(3.9, 8.1);
    for y in y0..=y1 {
        for x in x0..=x1 {
            assert_eq!(which(f.pixel(x, y)), Some((0, "front")), "hole at {x},{y}");
        }
    }
}

#[test]
fn slim_arms_are_narrower() {
    let arm_width = |model| {
        let f = Rasterizer::new().render(&face_id(), &flat(yaw(0.0), model));
        let (_, y) = screen(0.0, 2.0, 2.0);
        (0..W).filter(|&x| which(f.pixel(x, y)) == Some((2, "front"))).count()
    };
    let classic = arm_width(ArmModel::Classic);
    let slim = arm_width(ArmModel::Slim);
    assert!(classic > 0 && slim > 0);
    let ratio = slim as f32 / classic as f32;
    assert!((0.65..0.85).contains(&ratio), "slim {slim} / classic {classic}");
}

#[test]
fn second_layer_draws_over_the_base() {
    let skin = steve();
    let on = render_flat(&skin, yaw(0.0));
    let off = Rasterizer::new().render(&skin, &Scene { overlay: false, ..flat(yaw(0.0), ArmModel::Classic) });
    // the red head band sits on texel row 10 of the hat (row 2 of its front face)
    let (x, y) = screen(0.0, 13.75, 4.5);
    assert_eq!(on.pixel(x, y), [220, 60, 70, 255]);
    assert_ne!(off.pixel(x, y), [220, 60, 70, 255]);
    // the overlay is inflated: it covers pixels just outside the base head
    let (x, y) = screen(4.3, 12.0, 4.0);
    assert_eq!(off.pixel(x, y)[3], 0);
}

#[test]
fn translucent_overlay_texels_blend() {
    let skin = steve();
    let f = render_flat(&skin, yaw(0.0));
    // gold visor (alpha 128) over the head front row 4 (eyes row) -> mix
    let (x, y) = screen(0.0, 11.5, 4.5);
    let p = f.pixel(x, y);
    assert_eq!(p[3], 255);
    let gold = [240u8, 190, 60];
    assert!(p[0] < gold[0] && p[0] > 100, "blended {p:?}");
}

#[test]
fn legacy_skins_mirror_the_right_limbs() {
    let skin = SkinTexture::new(64, 32, steve_legacy_rgba()).unwrap();
    assert_eq!((skin.width, skin.height), (64, 64));
    let f = render_flat(&skin, yaw(0.0));
    for (xr, y) in [(-6.0, 4.0), (-2.0, -10.0)] {
        let (a, ay) = screen(xr, y, 2.0);
        let (b, by) = screen(-xr, y, 2.0);
        assert_eq!(f.pixel(a, ay)[3], 255);
        assert_eq!(f.pixel(b, by)[3], 255);
    }
    // the whole frame is left/right symmetric (texture mirrored, geometry mirrored)
    let mut asym = 0;
    for y in 0..H {
        for x in 0..W / 2 {
            if f.pixel(x, y)[3] != f.pixel(W - 1 - x, y)[3] {
                asym += 1;
            }
        }
    }
    assert!(asym < 200, "{asym} asymmetric pixels"); // legs sit at z=-0.1, tiny
}

#[test]
fn hd_skins_render_like_their_64px_source() {
    let lo = face_id();
    let mut hi = vec![0u8; 128 * 128 * 4];
    for y in 0..128 {
        for x in 0..128 {
            let s = (((y / 2) * 64 + x / 2) * 4) as usize;
            let d = ((y * 128 + x) * 4) as usize;
            hi[d..d + 4].copy_from_slice(&lo.rgba[s..s + 4]);
        }
    }
    let hi = SkinTexture::new(128, 128, hi).unwrap();
    let a = render_flat(&lo, yaw(-30.0));
    let b = render_flat(&hi, yaw(-30.0));
    let diff = a.pixels.chunks_exact(4).zip(b.pixels.chunks_exact(4)).filter(|(p, q)| p != q).count();
    assert!(diff < 50, "{diff} differing pixels");
}

#[test]
fn shading_darkens_side_faces_more_than_front() {
    let f = Rasterizer::new().render(&face_id(), &Scene::new(W, H, ArmModel::Classic, yaw(-40.0)));
    let flat = render_flat(&face_id(), yaw(-40.0));
    let (x, y) = screen(0.0, 12.0, 4.0);
    let p = f.pixel(x, y);
    let q = flat.pixel(x, y);
    assert!(p[0] <= q[0] && p[1] <= q[1] && p[2] <= q[2]);
}

#[test]
fn antialias_softens_edges() {
    let scene = Scene { antialias: true, ..Scene::new(W, H, ArmModel::Classic, yaw(-30.0)) };
    let f = Rasterizer::new().render(&steve(), &scene);
    let partial = f.pixels.chunks_exact(4).filter(|p| p[3] > 0 && p[3] < 255).count();
    assert!(partial > 50, "{partial} edge pixels");
}

#[test]
fn trait_renderer_follows_params() {
    let r = default_renderer();
    let p = RenderParams { width: 120, height: 160, yaw_deg: 30.0, time_s: 1.0, ..Default::default() };
    let a = r.render(&steve(), &p);
    let b = r.render(&steve(), &p);
    assert_eq!(a, b, "deterministic");
    let c = r.render(&steve(), &RenderParams { time_s: 2.5, ..p });
    assert_ne!(a, c, "idle animation moves the arms");
}

#[test]
fn walking_pose_moves_limbs() {
    let a = render_flat(&steve(), Pose::walking(90.0, 0.0, 0.0));
    let b = render_flat(&steve(), Pose::walking(90.0, 0.0, 0.2));
    assert_ne!(a, b);
}

#[test]
fn zero_sized_frames_are_fine() {
    let f = render(&steve(), ArmModel::Classic, &Pose::default(), 0, 0);
    assert!(f.pixels.is_empty());
}

/// Performance budget (release builds only: `cargo test --release -p espectral-skin3d`).
#[cfg(not(debug_assertions))]
#[test]
fn renders_300x400_well_under_4ms() {
    let skin = steve();
    let mut r = Rasterizer::new();
    let mut frame = RgbaFrame::transparent(300, 400);
    let mut times: Vec<f64> = (0..120)
        .map(|i| {
            let t0 = std::time::Instant::now();
            let pose = Pose::idle(i as f32 * 3.0, 8.0, i as f32 / 30.0);
            r.render_into(&skin, &Scene::new(300, 400, ArmModel::Classic, pose), &mut frame);
            t0.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    times.sort_by(|a, b| a.total_cmp(b));
    let median = times[times.len() / 2];
    assert!(median < 2.0, "median {median:.3} ms per 300x400 frame");
}
