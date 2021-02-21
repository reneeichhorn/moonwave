use moonwave_common::{Matrix4, Vector3};
use moonwave_core::{actor, Core};
use moonwave_shader::uniform;

use crate::Uniform;

#[uniform]
pub struct CameraUniform {
  projection: Matrix4<f32>,
  view: Matrix4<f32>,
  projection_view: Matrix4<f32>,
}

/// Used to tag the camera actor that is the scenes main / active camera
pub struct MainCameraTag;

#[actor]
pub struct Camera {
  pub uniform: Uniform<CameraUniform>,
  pub position: Vector3<f32>,
  pub target: Vector3<f32>,
  pub up: Vector3<f32>,
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
      fov_y: std::f32::consts::FRAC_PI_4,
      aspect: 1.0,
      position: Vector3::new(0.0, 0.0, 0.0),
      target: Vector3::new(0.0, 0.0, 0.0),
      up: Vector3::new(0.0, 1.0, 0.0),
      uniform: Uniform::new(
        CameraUniform {
          projection: Matrix4::identity(),
          view: Matrix4::identity(),
          projection_view: Matrix4::identity(),
        },
        core,
      )
      .await,
    }
  }
}

#[actor]
impl Camera {
  #[actor_tick(real)]
  pub async fn tick(&mut self) {
    // Build projection
    let projection = Matrix4::new_perspective(self.aspect, self.fov_y, self.z_near, self.z_far);

    // Build view matrix
    let view = Matrix4::look_at_lh(&self.position.into(), &self.target.into(), &self.up);

    // Build together
    let projection_view = projection * view;

    // Update uniform.
    let mut uniform = self.uniform.get_mut();
    uniform.view = view;
    uniform.projection = projection;
    uniform.projection_view = projection_view;
  }
}
