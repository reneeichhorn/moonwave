use std::f32::consts::PI;

pub use cgmath::Transform as MathTransform;
pub use cgmath::{
  dot, frustum, ortho, perspective, point1, point2, point3, vec1, vec2, vec3, vec4,
};
pub use cgmath::{
  AbsDiff, Basis2, Basis3, Decomposed, Deg, Euler, Matrix2, Matrix3, Matrix4, Ortho, Perspective,
  PerspectiveFov, Point1, Point2, Point3, Quaternion, Rad, Relative, Ulps, Vector1, Vector2,
  Vector3, Vector4,
};
pub use cgmath::{
  AbsDiffEq, Angle, Array, BaseFloat, BaseNum, Bounded, ElementWise, EuclideanSpace, InnerSpace,
  Matrix, MetricSpace, One, RelativeEq, Rotation, Rotation2, Rotation3, SquareMatrix, UlpsEq,
  VectorSpace, Zero,
};

mod color;
pub use color::*;

pub use bytemuck;

pub mod atomics;
pub use atomics::*;

use lazy_static::lazy_static;

#[rustfmt::skip]
lazy_static! {
  pub static ref MATRIX_NORMALIZER: Matrix4<f32> = {
    Matrix4::new(
      1.0, 0.0, 0.0, 0.0,
      0.0, 1.0, 0.0, 0.0,
      0.0, 0.0, 0.5, 0.0,
      0.0, 0.0, 0.5, 1.0,
    )
  };
}

pub fn correct_angle(rad: Rad<f32>) -> Rad<f32> {
  let value = rad.0;

  Rad(if value < 0.0 {
    2.0 * std::f32::consts::PI - (value.abs() % (2.0 * std::f32::consts::PI))
  } else if value > 2.0 * std::f32::consts::PI {
    value % (2.0 * PI)
  } else {
    value
  })
}

pub fn inv_lerp(a: f32, b: f32, v: f32) -> f32 {
  (v - a) / (b - a)
}
