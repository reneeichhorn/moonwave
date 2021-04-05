use moonwave_common::bytemuck::*;
use moonwave_core::Core;
use moonwave_render::{CommandEncoder, FrameGraphNode, FrameNodeValue};
use moonwave_resources::{Buffer, BufferUsage, ResourceRc};
use parking_lot::{RwLock, RwLockWriteGuard};
use std::sync::{
  atomic::{AtomicBool, Ordering},
  Arc,
};

#[derive(Clone)]
pub struct StagedBuffer<T: Sized> {
  content: Arc<RwLock<Vec<T>>>,
  is_dirty: Arc<AtomicBool>,
  staging_buffer: ResourceRc<Buffer>,
  buffer: ResourceRc<Buffer>,
}

impl<T: Sized + Pod> StagedBuffer<T> {
  pub fn new(length: u64, usage: BufferUsage) -> Self {
    let core = Core::get_instance();
    let size = (std::mem::size_of::<T>() * length as usize) as u64;
    let staging_buffer = core.create_buffer(
      size,
      false,
      BufferUsage::MAP_WRITE | BufferUsage::COPY_SRC,
      None,
    );

    let buffer = core.create_buffer(size, false, usage | BufferUsage::COPY_DST, None);

    Self {
      staging_buffer,
      buffer,
      content: Arc::new(RwLock::new(Vec::with_capacity(length as usize))),
      is_dirty: Arc::new(AtomicBool::new(false)),
    }
  }

  pub fn get_mut(&self) -> RwLockWriteGuard<Vec<T>> {
    self.is_dirty.store(true, Ordering::Relaxed);
    self.content.write()
  }

  pub fn get_accessor(&self) -> StagedBufferAccessor {
    let content = if self.is_dirty.swap(false, Ordering::Relaxed) {
      let out = moonwave_common::bytemuck::cast_slice(&*self.content.read()).to_vec();
      Some(out)
    } else {
      None
    };

    StagedBufferAccessor {
      content,
      buffer: self.buffer.clone(),
      staging_buffer: self.staging_buffer.clone(),
    }
  }
}

pub struct StagedBufferAccessor {
  content: Option<Vec<u8>>,
  staging_buffer: ResourceRc<Buffer>,
  buffer: ResourceRc<Buffer>,
}

impl StagedBufferAccessor {
  pub fn get_resources(&self, cmd: &mut CommandEncoder) -> &ResourceRc<Buffer> {
    if let Some(data) = &self.content {
      // Update staging buffer.
      cmd.write_buffer(&self.staging_buffer, &data);

      // Update actual buffer
      cmd.copy_buffer_to_buffer(&self.staging_buffer, &self.buffer, data.len() as u64)
    }

    &self.buffer
  }
}
