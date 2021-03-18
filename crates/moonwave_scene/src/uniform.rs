use moonwave_core::Core;
use moonwave_render::{CommandEncoder, FrameGraphNode, FrameNodeValue};
use moonwave_resources::{BindGroup, BindGroupDescriptor, Buffer, BufferUsage, ResourceRc};
use moonwave_shader::UniformStruct;
use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::sync::{
  atomic::{AtomicBool, Ordering},
  Arc,
};

use crate::MATERIAL_UNIFORM_LAYOUT;

#[derive(Clone)]
pub struct Uniform<T: UniformStruct> {
  content: Arc<RwLock<T>>,
  staging_buffer: ResourceRc<Buffer>,
  is_dirty: Arc<AtomicBool>,
  resources: Arc<PubUniformResources>,
}

impl<T: UniformStruct + Send + Sync + 'static> Uniform<T> {
  pub fn new(initial: T) -> Self {
    let size = initial.generate_raw_u8().len() as u64;

    let core = Core::get_instance();
    let staging_buffer = core.create_buffer(
      size,
      false,
      BufferUsage::MAP_WRITE | BufferUsage::COPY_SRC,
      None,
    );

    let buffer = core.create_buffer(
      size,
      false,
      BufferUsage::UNIFORM | BufferUsage::COPY_DST,
      None,
    );

    let bind_group = core.create_bind_group(
      BindGroupDescriptor::new(MATERIAL_UNIFORM_LAYOUT.clone())
        .add_buffer_binding(0, buffer.clone()),
    );

    Self {
      resources: Arc::new(PubUniformResources { buffer, bind_group }),
      staging_buffer,
      content: Arc::new(RwLock::new(initial)),
      is_dirty: Arc::new(AtomicBool::new(true)),
    }
  }

  pub fn get_mut(&self) -> RwLockWriteGuard<T> {
    self.is_dirty.store(true, Ordering::Relaxed);
    self.content.write()
  }

  pub fn get(&self) -> RwLockReadGuard<T> {
    self.content.read()
  }

  pub fn get_bind_group(&self) -> ResourceRc<BindGroup> {
    self.resources.bind_group.clone()
  }

  pub fn as_generic(&self) -> GenericUniform {
    let content = if self.is_dirty.swap(false, Ordering::Relaxed) {
      Some(self.content.read().generate_raw_u8())
    } else {
      None
    };

    GenericUniform {
      content,
      resources: self.resources.clone(),
      staging_buffer: self.staging_buffer.clone(),
    }
  }
}

pub struct GenericUniform {
  content: Option<Vec<u8>>,
  staging_buffer: ResourceRc<Buffer>,
  resources: Arc<PubUniformResources>,
}

impl GenericUniform {
  pub fn get_resources(&self, cmd: &mut CommandEncoder) -> &PubUniformResources {
    if let Some(data) = &self.content {
      // Update staging buffer.
      cmd.write_buffer(&self.staging_buffer, &data);

      // Update actual buffer
      cmd.copy_buffer_to_buffer(
        &self.staging_buffer,
        &self.resources.buffer,
        data.len() as u64,
      )
    }

    &self.resources
  }
}

pub struct PubUniformResources {
  pub buffer: ResourceRc<Buffer>,
  pub bind_group: ResourceRc<BindGroup>,
}

pub struct DynamicUniformNode<T: UniformStruct> {
  content: Option<Arc<RwLock<T>>>,
  buffer: ResourceRc<Buffer>,
  buffer_staging: ResourceRc<Buffer>,
  bind_group: ResourceRc<BindGroup>,
}

impl<T: UniformStruct> DynamicUniformNode<T> {
  pub const OUTPUT_BUFFER: usize = 0;
  pub const OUTPUT_BIND_GROUP: usize = 0;
}

impl<T: UniformStruct + Send + Sync + 'static> FrameGraphNode for DynamicUniformNode<T> {
  fn execute(
    &self,
    _inputs: &[Option<FrameNodeValue>],
    outputs: &mut [Option<FrameNodeValue>],
    encoder: &mut CommandEncoder,
  ) {
    if let Some(content) = &self.content {
      // Build buffer
      let data = content.read().generate_raw_u8();

      // Update staging buffer.
      encoder.write_buffer(&self.buffer_staging, &data);

      // Update actual buffer
      encoder.copy_buffer_to_buffer(&self.buffer_staging, &self.buffer, data.len() as u64)
    }

    // Set buffer as output of node.
    outputs[Self::OUTPUT_BUFFER] = Some(FrameNodeValue::Buffer(self.buffer.clone()));
    outputs[Self::OUTPUT_BIND_GROUP] = Some(FrameNodeValue::BindGroup(self.bind_group.clone()));
  }
}
