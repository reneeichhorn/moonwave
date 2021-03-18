pub use cgmath::prelude::*;
pub use cgmath::*;

mod color;
pub use color::*;

pub use bytemuck;

pub mod atomics;

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
