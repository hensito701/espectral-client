//! Golden images: render the fixture skin in fixed poses and compare with the
//! committed PNGs in tests/golden/ (small tolerance for float differences
//! across platforms / compilers). Regenerate with `UPDATE_GOLDEN=1 cargo test
//! -p espectral-skin3d --test golden` after an intended visual change, and
//! look at the new PNGs before committing them.

mod common;

use std::path::PathBuf;

use espectral_skin3d::fixtures::{steve, steve_legacy_rgba};
use espectral_skin3d::{ArmModel, Pose, Rasterizer, Scene, SkinTexture};

fn cases() -> Vec<(&'static str, SkinTexture, Scene)> {
    let s = |model, pose| Scene::new(150, 200, model, pose);
    let legacy = SkinTexture::new(64, 32, steve_legacy_rgba()).unwrap();
    vec![
        ("front", steve(), s(ArmModel::Classic, Pose::idle(0.0, 0.0, 0.0))),
        ("three-quarter", steve(), s(ArmModel::Classic, Pose::idle(-35.0, 10.0, 0.0))),
        ("back-slim", steve(), s(ArmModel::Slim, Pose::idle(160.0, 5.0, 0.0))),
        ("above", steve(), s(ArmModel::Classic, Pose::idle(30.0, 55.0, 0.0))),
        ("below", steve(), s(ArmModel::Classic, Pose::idle(-20.0, -40.0, 0.0))),
        ("walking", steve(), s(ArmModel::Classic, Pose::walking(70.0, 8.0, 0.3))),
        ("legacy", legacy, s(ArmModel::Classic, Pose::idle(-25.0, 8.0, 0.0))),
        ("no-overlay", steve(), Scene { overlay: false, ..s(ArmModel::Classic, Pose::idle(-25.0, 8.0, 0.0)) }),
        ("antialias", steve(), Scene { antialias: true, ..s(ArmModel::Slim, Pose::idle(-25.0, 8.0, 0.0)) }),
    ]
}

#[test]
fn golden_images() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    let mut r = Rasterizer::new();
    let mut failures = Vec::new();
    for (name, skin, scene) in cases() {
        let frame = r.render(&skin, &scene);
        let path = dir.join(format!("{name}.png"));
        if update {
            common::write_png(&path, frame.width, frame.height, &frame.pixels);
            continue;
        }
        let Some((w, h, want)) = common::read_png(&path) else {
            failures.push(format!("{name}: missing {} (run with UPDATE_GOLDEN=1)", path.display()));
            continue;
        };
        assert_eq!((w, h), (frame.width, frame.height), "{name}: size");
        let bad = want
            .chunks_exact(4)
            .zip(frame.pixels.chunks_exact(4))
            .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 12))
            .count();
        let limit = (w * h) as usize / 200; // 0.5 %
        if bad > limit {
            let out = std::env::temp_dir().join(format!("skin3d-golden-{name}.png"));
            common::write_png(&out, frame.width, frame.height, &frame.pixels);
            failures.push(format!("{name}: {bad} pixels differ (limit {limit}); got {}", out.display()));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
