use std::num::NonZeroU64;

use moonwave_render::execute_wgpu_async;
use moonwave_resources::{Buffer, ResourceRc};

use crate::Core;

pub struct StagingBelt {
  raw_belt: wgpu::util::StagingBelt,
}

impl StagingBelt {
  pub fn new(chunk_size: usize) -> Self {
    Self {
      raw_belt: wgpu::util::StagingBelt::new(chunk_size as u64),
    }
  }

  pub fn write_immediate(&mut self, target: &ResourceRc<Buffer>, offset: u64, data: &[u8]) {
    let core = Core::get_instance();
    core.exec_with_encoder(|cmd| {
      let padding_required = data.len() as u64 % wgpu::COPY_BUFFER_ALIGNMENT;

      let mut buffer_mut = self.raw_belt.write_buffer(
        cmd.get_raw(),
        target.get_raw(),
        offset,
        NonZeroU64::new(data.len() as u64 + padding_required).unwrap(),
        &core.device,
      );
      buffer_mut[..data.len()].copy_from_slice(data);
      drop(buffer_mut);

      self.raw_belt.finish();
    });

    let fut = self.raw_belt.recall();
    execute_wgpu_async(&core.device, fut);
  }
}
