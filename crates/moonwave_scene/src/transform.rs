use std::{sync::atomic::AtomicBool, unimplemented};

use legion::maybe_changed;
use moonwave_common::*;
use moonwave_core::*;
use moonwave_shader::uniform;

use crate::Uniform;

static REGISTERED_SYSTEM: std::sync::Once = std::sync::Once::new();

#[uniform]
pub struct TransformUniform {
  pub matrix: Matrix4<f32>,
}

#[derive(Debug)]
pub struct TransformInner {
  pub space: TransformSpace,
  pub opt: TransformOptimization,

  pub position: Vector3<f32>,
  pub rotation: Vector3<f32>,
  pub scale: Vector3<f32>,
}

pub struct Transform {
  pub(crate) uniform: Option<Uniform<TransformUniform>>,
  pub inner: TransformInner,
  dirty: AtomicBool,
}

impl Transform {
  pub fn new_static(position: Vector3<f32>, rotation: Vector3<f32>, scale: Vector3<f32>) -> Self {
    REGISTERED_SYSTEM.call_once(|| {
      let core = Core::get_instance();
      core.get_world().add_system_to_stage(
        UpdateTransformUniformSystem,
        SystemStage::RenderingPreperations,
      )
    });

    Self {
      uniform: None,
      inner: TransformInner {
        opt: TransformOptimization::Static,
        space: TransformSpace::World,
        position,
        rotation,
        scale,
      },
      dirty: AtomicBool::new(false),
    }
  }

  pub fn new_dynamic(position: Vector3<f32>, rotation: Vector3<f32>, scale: Vector3<f32>) -> Self {
    REGISTERED_SYSTEM.call_once(|| {
      let core = Core::get_instance();
      core.get_world().add_system_to_stage(
        UpdateTransformUniformSystem,
        SystemStage::RenderingPreperations,
      )
    });

    Self {
      uniform: None,
      inner: TransformInner {
        opt: TransformOptimization::Dynamic,
        space: TransformSpace::World,
        position,
        rotation,
        scale,
      },
      dirty: AtomicBool::new(true),
    }
  }

  pub fn new() -> Self {
    REGISTERED_SYSTEM.call_once(|| {
      let core = Core::get_instance();
      core.get_world().add_system_to_stage(
        UpdateTransformUniformSystem,
        SystemStage::RenderingPreperations,
      )
    });

    Self {
      uniform: Some(Uniform::new(TransformUniform {
        matrix: Matrix4::identity(),
      })),
      inner: TransformInner {
        opt: TransformOptimization::Dynamic,
        space: TransformSpace::World,
        position: Vector3::new(0.0, 0.0, 0.0),
        rotation: Vector3::new(0.0, 0.0, 0.0),
        scale: Vector3::new(1.0, 1.0, 1.0),
      },
      dirty: AtomicBool::new(true),
    }
  }

  pub fn calculate_transform_matrix(&self) -> Matrix4<f32> {
    let translation = Matrix4::from_translation(self.inner.position);
    let rotation = Matrix4::from_angle_x(Rad(self.inner.rotation.x))
      * Matrix4::from_angle_y(Rad(self.inner.rotation.y))
      * Matrix4::from_angle_z(Rad(self.inner.rotation.z));
    let scale =
      Matrix4::from_nonuniform_scale(self.inner.scale.x, self.inner.scale.y, self.inner.scale.z);
    translation * rotation * scale
  }

  #[inline]
  pub fn get(&self) -> &TransformInner {
    &self.inner
  }

  #[inline]
  pub fn get_mut(&mut self) -> &mut TransformInner {
    std::assert!(
      !matches!(self.inner.opt, TransformOptimization::Static),
      "Tried to mutably access an static transform component."
    );

    self.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
    &mut self.inner
  }
}

/// Defines the space the transform should be placed in.
#[derive(PartialEq, Eq, Debug)]
pub enum TransformSpace {
  /// Overall world space.
  World,
  /// Relative to its direct parent. Falls back to World space if direct parent has no Model component.
  RelativeDirect,
  /// Relative to its next parent that also has a Model component.
  RelativeNext,
}

/// Optimization property.
#[derive(PartialEq, Eq, Debug)]
pub enum TransformOptimization {
  /// Fully static won't change after creation at all.
  Static,
  /// Fully dynamic changes can happen every single frame.
  Dynamic,
}

#[system(par_for_each)]
#[filter(maybe_changed::<Transform>())]
pub fn update_transform_uniforms(model: &Transform) {
  if !model
    .dirty
    .swap(false, std::sync::atomic::Ordering::Relaxed)
  {
    return;
  }

  // Update uniform
  if let Some(uniform) = &model.uniform {
    if model.inner.space != TransformSpace::World {
      unimplemented!("Only ModelSpace::World supported right now :(")
    }

    // Build matrix
    let matrix = model.calculate_transform_matrix();

    uniform.get_mut().matrix = matrix;
  }
}

struct UpdateTransformUniformSystem;
impl SystemFactory for UpdateTransformUniformSystem {
  fn create_system(&self) -> WrappedSystem {
    WrappedSystem(Box::new(update_transform_uniforms_system()))
  }
}
