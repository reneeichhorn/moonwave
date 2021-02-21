use moonwave_common::{Matrix4, Vector3};
use moonwave_core::{actor, Core};
use moonwave_shader::{uniform, UniformStruct};

use crate::Uniform;

#[uniform]
pub struct CameraUniform {
  projection: Matrix4<f32>,
  view: Matrix4<f32>,
}

#[actor]
pub struct Camera {
  uniform: Uniform<CameraUniform>,
  pub position: Vector3<f32>,
  pub rotation: Vector3<f32>,
  pub aspect: f32,
  pub fov_y: f32,
  z_near: f32,
  z_far: f32,
}

impl Camera {
  pub async fn new(core: &Core) -> Self {
    Self {
      z_far: 100.0,
      z_near: 0.01,
      fov_y: 45.0,
      aspect: 1.0,
      position: Vector3::new(0.0, 0.0, 0.0),
      rotation: Vector3::new(0.0, 0.0, 0.0),
      uniform: Uniform::new(
        CameraUniform {
          projection: Matrix4::identity(),
          view: Matrix4::identity(),
        },
        core,
      )
      .await,
    }
  }
}

#[actor]
impl Camera {}
