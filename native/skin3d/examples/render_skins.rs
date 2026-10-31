//! Renders a skin to PNGs and benchmarks the rasterizer.
//!
//! ```sh
//! cargo run --release -p espectral-skin3d --example render_skins -- OUT_DIR [skin.png] [--slim]
//! ```
//!
//! Without a skin the built-in fixture is used. Writes `OUT_DIR/pose-*.png`
//! (single frames), `OUT_DIR/turntable.png` (a 12-frame strip), the fixture
//! textures (`fixture-skin*.png`, when no skin is given) and prints the
//! mean time of a 300x400 frame with and without 2x2 antialiasing.

#[path = "../tests/common/mod.rs"]
mod common;

use std::path::PathBuf;
use std::time::Instant;

use espectral_skin3d::{fixtures, ArmModel, Pose, Rasterizer, RgbaFrame, Scene, SkinTexture};

fn main() {
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(args.next().unwrap_or_else(|| "skin3d-out".into()));
    let mut skin_path = None;
    let mut model = ArmModel::Classic;
    for a in args {
        if a == "--slim" {
            model = ArmModel::Slim;
        } else {
            skin_path = Some(PathBuf::from(a));
        }
    }
    std::fs::create_dir_all(&out).expect("create output dir");
    let skin = match &skin_path {
        Some(p) => {
            let (w, h, rgba) = common::read_png(p).expect("readable 8-bit PNG");
            SkinTexture::new(w, h, rgba).expect("64x64 / 64x32 skin")
        }
        None => {
            // also export the fixture texture itself (handy as a test skin elsewhere)
            common::write_png(&out.join("fixture-skin.png"), 64, 64, &fixtures::steve_rgba());
            common::write_png(&out.join("fixture-skin-legacy.png"), 64, 32, &fixtures::steve_legacy_rgba());
            fixtures::steve()
        }
    };

    let mut r = Rasterizer::new();
    let poses = [
        ("front", Pose::idle(0.0, 0.0, 0.0)),
        ("three-quarter", Pose::idle(-30.0, 10.0, 0.0)),
        ("side", Pose::idle(90.0, 0.0, 0.0)),
        ("back", Pose::idle(180.0, 8.0, 0.0)),
        ("above", Pose::idle(20.0, 60.0, 0.0)),
        ("walking", Pose::walking(60.0, 8.0, 0.3)),
    ];
    for (name, pose) in poses {
        let f = r.render(&skin, &Scene { antialias: true, ..Scene::new(300, 400, model, pose) });
        common::write_png(&out.join(format!("pose-{name}.png")), f.width, f.height, &f.pixels);
    }

    // turntable strip: 12 frames, 30° apart, idle animation running
    let (fw, fh, n) = (150u32, 200u32, 12u32);
    let mut strip = RgbaFrame::transparent(fw * n, fh);
    for i in 0..n {
        let pose = Pose::idle(i as f32 * 30.0, 8.0, i as f32 * 0.25);
        let f = r.render(&skin, &Scene { antialias: true, ..Scene::new(fw, fh, model, pose) });
        for y in 0..fh {
            let src = (y * fw * 4) as usize;
            let dst = ((y * fw * n + i * fw) * 4) as usize;
            strip.pixels[dst..dst + (fw * 4) as usize].copy_from_slice(&f.pixels[src..src + (fw * 4) as usize]);
        }
    }
    common::write_png(&out.join("turntable.png"), strip.width, strip.height, &strip.pixels);

    // benchmark: 300x400, rotating, one core
    for aa in [false, true] {
        let mut frame = RgbaFrame::transparent(300, 400);
        let iters = 400;
        let t0 = Instant::now();
        for i in 0..iters {
            let pose = Pose::idle(i as f32 * 3.0, 8.0, i as f32 / 30.0);
            r.render_into(&skin, &Scene { antialias: aa, ..Scene::new(300, 400, model, pose) }, &mut frame);
        }
        let per = t0.elapsed().as_secs_f64() * 1000.0 / iters as f64;
        println!("300x400 frame{}: {per:.3} ms", if aa { " (2x2 antialias)" } else { "" });
    }
    println!("wrote {}", out.display());
}
