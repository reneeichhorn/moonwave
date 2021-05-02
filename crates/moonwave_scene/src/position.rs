use std::{sync::atomic::AtomicBool, unimplemented};

use legion::IntoQuery;
use legion::{maybe_changed, world::SubWorld};
use moonwave_common::*;
use moonwave_core::*;
use moonwave_shader::uniform;

use crate::Uniform;

static REGISTERED_SYSTEM: std::sync::Once = std::sync::Once::new();

#[uniform]
pub struct ModelUniform {
  pub matrix: Matrix4<f32>,
}

pub struct ModelInner {
  pub space: ModelSpace,
  pub position: Vector3<f32>,
  pub rotation: Vector3<f32>,
  pub scale: Vector3<f32>,
}

pub struct Model {
  pub(crate) uniform: Uniform<ModelUniform>,
  inner: ModelInner,
  dirty: AtomicBool,
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
      inner: ModelInner {
        space: ModelSpace::World,
        position: Vector3::new(0.0, 0.0, 0.0),
        rotation: Vector3::new(0.0, 0.0, 0.0),
        scale: Vector3::new(1.0, 1.0, 1.0),
      },
      dirty: AtomicBool::new(true),
    }
  }

  #[inline]
  pub fn get(&self) -> &ModelInner {
    &self.inner
  }

  #[inline]
  pub fn get_mut(&mut self) -> &mut ModelInner {
    self.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
    &mut self.inner
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

#[system(par_for_each)]
#[filter(maybe_changed::<Model>())]
pub fn update_model_uniforms(model: &Model) {
  if !model
    .dirty
    .swap(false, std::sync::atomic::Ordering::Relaxed)
  {
    return;
  }

  if model.inner.space != ModelSpace::World {
    unimplemented!("Only ModelSpace::World supported right now :(")
  }

  // Build matrix
  let translation = Matrix4::from_translation(model.inner.position);
  let rotation = Matrix4::from_angle_x(Rad(model.inner.rotation.x))
    * Matrix4::from_angle_y(Rad(model.inner.rotation.y))
    * Matrix4::from_angle_z(Rad(model.inner.rotation.z));
  let scale = Matrix4::from_nonuniform_scale(
    model.inner.scale.x,
    model.inner.scale.y,
    model.inner.scale.z,
  );
  let matrix = translation * rotation * scale;

  // Update uniform
  model.uniform.get_mut().matrix = matrix;
}

struct UpdateModelUniformSystem;
impl SystemFactory for UpdateModelUniformSystem {
  fn create_system(&self) -> WrappedSystem {
    WrappedSystem(Box::new(update_model_uniforms_system()))
  }
}
