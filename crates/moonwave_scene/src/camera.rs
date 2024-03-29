use legion::systems::ParallelRunnable;
use moonwave_common::*;
use moonwave_core::{system, Core, SystemStage};
use moonwave_shader::uniform;

use crate::Uniform;

#[uniform]
pub struct CameraUniform {
  pub(crate) projection: Matrix4<f32>,
  pub(crate) view: Matrix4<f32>,
  pub(crate) projection_view: Matrix4<f32>,
  position: Vector3<f32>,
}

/// Used to tag the camera actor that is the scenes main / active camera
pub struct MainCameraTag;

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

static REGISTERED_SYSTEM: std::sync::Once = std::sync::Once::new();

impl Camera {
  pub fn new() -> Self {
    REGISTERED_SYSTEM.call_once(|| {
      let core = Core::get_instance();
      core.get_world().add_system_to_stage(
        || -> Box<dyn ParallelRunnable> { Box::new(update_camera_matrices_system()) },
        SystemStage::RenderingPreperations,
      )
    });

    Self {
      z_far: 100.0,
      z_near: 0.01,
      fov_y: std::f32::consts::FRAC_PI_4,
      aspect: 1.0,
      position: Vector3::new(0.0, 0.0, 0.0),
      target: Vector3::new(0.0, 0.0, 1.0),
      up: Vector3::new(0.0, 1.0, 0.0),
      uniform: Uniform::new(CameraUniform {
        projection: Matrix4::identity(),
        view: Matrix4::identity(),
        projection_view: Matrix4::identity(),
        position: Vector3::zero(),
      }),
    }
  }
  pub fn calculate_frustum_planes(&self, planes: &mut [Vector4<f32>; 6]) {
    // Extract planes from view projection.
    let vp = self.uniform.get().projection_view;
    for i in 0..4 {
      planes[0][i] = vp[i][3] + vp[i][0]
    } // right
    for i in 0..4 {
      planes[1][i] = vp[i][3] - vp[i][0]
    } // left
    for i in 0..4 {
      planes[2][i] = vp[i][3] + vp[i][1]
    } // top
    for i in 0..4 {
      planes[3][i] = vp[i][3] - vp[i][1]
    } // bottom
    for i in 0..4 {
      planes[4][i] = vp[i][3] + vp[i][2]
    } // far
    for i in 0..4 {
      planes[5][i] = vp[i][3] - vp[i][2]
    } // near

    // Normalize planes
    for plane in planes {
      *plane = plane.normalize();
    }
  }
}

#[system(par_for_each)]
fn update_camera_matrices(camera: &Camera) {
  // Build projection
  let projection = perspective(
    Rad(camera.fov_y),
    camera.aspect,
    camera.z_near,
    camera.z_far,
  );

  // Build view matrix
  let view = Matrix4::look_at_rh(
    Point3::from_vec(camera.position),
    Point3::from_vec(camera.target),
    camera.up,
  );

  // Build together
  let projection_view = projection * view;

  // Update uniform.
  let mut uniform = camera.uniform.get_mut();
  uniform.view = view;
  uniform.projection = projection;
  uniform.projection_view = projection_view;
  uniform.position = camera.position;
}
