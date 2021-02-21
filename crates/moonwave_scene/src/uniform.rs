use moonwave_core::Core;
use moonwave_render::{CommandEncoder, FrameGraphNode, FrameNodeValue, Index};
use moonwave_resources::{Buffer, BufferUsage, ResourceRc};
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
}

impl<T: UniformStruct + Send + Sync + 'static> Uniform<T> {
  pub async fn new(initial: T, core: &Core) -> Self {
    let size = initial.generate_raw_u8().len() as u64;

    let staging_buffer = core
      .create_buffer(
        size,
        false,
        BufferUsage::MAP_WRITE | BufferUsage::COPY_SRC,
        None,
      )
      .await;

    let buffer = core
      .create_buffer(
        size,
        false,
        BufferUsage::UNIFORM | BufferUsage::COPY_DST,
        None,
      )
      .await;

    Self {
      buffer,
      staging_buffer,
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

  pub fn lazy_get_frame_node(&self, core: &Core) -> Index {
    let mut node = self.frame_node.write();
    if let Some(node) = *node {
      return node;
    }

    let name = T::generate_name();
    let node_index = core.get_frame_graph().add_node(
      DynamicUniformNode {
        buffer: self.buffer.clone(),
        buffer_staging: self.staging_buffer.clone(),
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
}

impl<T: UniformStruct> DynamicUniformNode<T> {
  pub const OUTPUT_BUFFER: usize = 0;
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
  }
}
