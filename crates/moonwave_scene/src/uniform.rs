use moonwave_core::{BindGroupLayoutSingleton, Core};
use moonwave_render::{CommandEncoder, FrameGraphNode, FrameNodeValue, Index};
use moonwave_resources::{BindGroup, BindGroupDescriptor, Buffer, BufferUsage, ResourceRc};
use moonwave_shader::UniformStruct;
use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::sync::{
  atomic::{AtomicBool, Ordering},
  Arc,
};

#[derive(Clone)]
pub struct Uniform<T: UniformStruct> {
  content: Arc<RwLock<T>>,
  frame_node: Arc<RwLock<Option<Index>>>,
  staging_buffer: ResourceRc<Buffer>,
  buffer: ResourceRc<Buffer>,
  is_dirty: Arc<AtomicBool>,
  bind_group: ResourceRc<BindGroup>,
}

impl<T: UniformStruct + BindGroupLayoutSingleton + Send + Sync + 'static> Uniform<T> {
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

    let bind_group_layout = T::get_bind_group_lazy();
    let bind_group = core.create_bind_group(
      BindGroupDescriptor::new(bind_group_layout).add_buffer_binding(0, buffer.clone()),
    );

    Self {
      buffer,
      staging_buffer,
      bind_group,
      content: Arc::new(RwLock::new(initial)),
      frame_node: Arc::new(RwLock::new(None)),
      is_dirty: Arc::new(AtomicBool::new(true)),
    }
  }

  pub fn get_mut(&self) -> RwLockWriteGuard<T> {
    self.content.write()
  }

  pub fn get(&self) -> RwLockReadGuard<T> {
    self.content.read()
  }

  pub fn get_bind_group(&self) -> ResourceRc<BindGroup> {
    self.bind_group.clone()
  }

  pub fn lazy_get_frame_node(&self) -> Index {
    let mut node = self.frame_node.write();
    if let Some(node) = *node {
      return node;
    }

    let name = T::generate_name();
    let node_index = Core::get_instance().get_frame_graph().add_node(
      DynamicUniformNode {
        buffer: self.buffer.clone(),
        buffer_staging: self.staging_buffer.clone(),
        bind_group: self.bind_group.clone(),
        content: if self.is_dirty.load(Ordering::Relaxed) {
          Some(self.content.clone())
        } else {
          None
        },
      },
      format!("Uniform_{}", name).as_str(),
    );
    *node = Some(node_index);
    node_index
  }
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
