use std::f32::consts::PI;

pub use cgmath::prelude::*;
pub use cgmath::*;

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
