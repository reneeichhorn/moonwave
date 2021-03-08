use std::unimplemented;

use legion::world::SubWorld;
use legion::IntoQuery;
use moonwave_common::{Matrix4, Vector3};
use moonwave_core::*;
use moonwave_shader::uniform;

use crate::Uniform;

static REGISTERED_SYSTEM: std::sync::Once = std::sync::Once::new();

#[uniform]
pub struct ModelUniform {
  matrix: Matrix4<f32>,
}

pub struct Model {
  pub space: ModelSpace,
  pub position: Vector3<f32>,
  pub rotation: Vector3<f32>,
  pub scale: Vector3<f32>,
  pub(crate) uniform: Uniform<ModelUniform>,
}

impl Model {
  pub fn new() -> Self {
    REGISTERED_SYSTEM.call_once(|| {
      let core = Core::get_instance();
      core
        .get_world()
        .add_system_to_stage(UpdateModelUniformSystem, SystemStage::RenderingPreperations)
    });

    Self {
      uniform: Uniform::new(ModelUniform {
        matrix: Matrix4::identity(),
      }),
      space: ModelSpace::World,
      position: Vector3::new(0.0, 0.0, 0.0),
      rotation: Vector3::new(0.0, 0.0, 0.0),
      scale: Vector3::new(1.0, 1.0, 1.0),
    }
  }
}

/// Defines the space the model should be placed in.
#[derive(PartialEq, Eq)]
pub enum ModelSpace {
  /// Overall world space.
  World,
  /// Relative to its direct parent. Falls back to World space if direct parent has no Model component.
  RelativeDirect,
  /// Relative to its next parent that also has a Model component.
  RelativeNext,
}

#[system]
#[write_component(Model)]
pub fn update_model_uniforms(world: &mut SubWorld) {
  let mut query = <&mut Model>::query();
  for model in query.iter_mut(world) {
    if model.space != ModelSpace::World {
      unimplemented!("Only ModelSpace::World supported right now :(")
    }

    // Build matrix
    let translation = Matrix4::new_translation(&model.position);
    let rotation = Matrix4::new_rotation(model.rotation);
    let scale = Matrix4::new_nonuniform_scaling(&model.scale);
    let matrix = translation * rotation * scale;

    // Update uniform
    model.uniform.get_mut().matrix = matrix;
  }
}
struct UpdateModelUniformSystem;
impl SystemFactory for UpdateModelUniformSystem {
  fn create_system(&self) -> WrappedSystem {
    WrappedSystem(Box::new(update_model_uniforms_system()))
  }
}
