//! Tiny 3D math: just what a box-model rasterizer needs (f32, right-handed, y up).

use std::ops::{Add, Mul, Neg, Sub};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct V3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

pub const fn v3(x: f32, y: f32, z: f32) -> V3 {
    V3 { x, y, z }
}

impl V3 {
    pub fn dot(self, o: V3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    pub fn len(self) -> f32 {
        self.dot(self).sqrt()
    }
    pub fn normalized(self) -> V3 {
        let l = self.len();
        if l > 0.0 {
            self * (1.0 / l)
        } else {
            self
        }
    }
}

impl Add for V3 {
    type Output = V3;
    fn add(self, o: V3) -> V3 {
        v3(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl Sub for V3 {
    type Output = V3;
    fn sub(self, o: V3) -> V3 {
        v3(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl Mul<f32> for V3 {
    type Output = V3;
    fn mul(self, s: f32) -> V3 {
        v3(self.x * s, self.y * s, self.z * s)
    }
}
impl Neg for V3 {
    type Output = V3;
    fn neg(self) -> V3 {
        v3(-self.x, -self.y, -self.z)
    }
}

/// Row-major 3x3 rotation matrix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct M3(pub [[f32; 3]; 3]);

impl M3 {
    pub const IDENTITY: M3 = M3([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);

    /// Rotation around +X (three.js convention: y -> z for positive angles).
    pub fn rot_x(a: f32) -> M3 {
        let (s, c) = a.sin_cos();
        M3([[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]])
    }
    /// Rotation around +Y (z -> x for positive angles).
    pub fn rot_y(a: f32) -> M3 {
        let (s, c) = a.sin_cos();
        M3([[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]])
    }
    /// Rotation around +Z (x -> y for positive angles).
    pub fn rot_z(a: f32) -> M3 {
        let (s, c) = a.sin_cos();
        M3([[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]])
    }
    /// three.js Euler order "XYZ": R = Rx * Ry * Rz.
    pub fn euler_xyz(x: f32, y: f32, z: f32) -> M3 {
        M3::rot_x(x).mul(M3::rot_y(y)).mul(M3::rot_z(z))
    }

    pub fn mul(self, o: M3) -> M3 {
        let a = &self.0;
        let b = &o.0;
        let mut r = [[0.0f32; 3]; 3];
        for (i, row) in r.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
            }
        }
        M3(r)
    }

    pub fn apply(&self, v: V3) -> V3 {
        let m = &self.0;
        v3(
            m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
            m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
            m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
        )
    }
}

/// Rigid transform: `r * v + t`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Xf {
    pub r: M3,
    pub t: V3,
}

impl Xf {
    #[cfg(test)]
    pub const IDENTITY: Xf = Xf { r: M3::IDENTITY, t: v3(0.0, 0.0, 0.0) };

    pub fn translate(t: V3) -> Xf {
        Xf { r: M3::IDENTITY, t }
    }
    pub fn rotate(r: M3) -> Xf {
        Xf { r, t: V3::default() }
    }
    /// `self ∘ o` (apply `o` first).
    pub fn then_inner(self, o: Xf) -> Xf {
        Xf { r: self.r.mul(o.r), t: self.r.apply(o.t) + self.t }
    }
    pub fn point(&self, v: V3) -> V3 {
        self.r.apply(v) + self.t
    }
    pub fn dir(&self, v: V3) -> V3 {
        self.r.apply(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: V3, b: V3) -> bool {
        (a - b).len() < 1e-5
    }

    #[test]
    fn rotations_follow_threejs_handedness() {
        let q = std::f32::consts::FRAC_PI_2;
        assert!(close(M3::rot_y(q).apply(v3(0.0, 0.0, 1.0)), v3(1.0, 0.0, 0.0)));
        assert!(close(M3::rot_x(q).apply(v3(0.0, 1.0, 0.0)), v3(0.0, 0.0, 1.0)));
        assert!(close(M3::rot_z(q).apply(v3(1.0, 0.0, 0.0)), v3(0.0, 1.0, 0.0)));
    }

    #[test]
    fn compose_applies_inner_first() {
        let a = Xf::translate(v3(1.0, 0.0, 0.0));
        let b = Xf::rotate(M3::rot_z(std::f32::consts::FRAC_PI_2));
        // rotate (1,0,0) -> (0,1,0), then translate -> (1,1,0)
        assert!(close(a.then_inner(b).point(v3(1.0, 0.0, 0.0)), v3(1.0, 1.0, 0.0)));
    }
}
