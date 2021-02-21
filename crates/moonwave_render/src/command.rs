use std::{
  mem::ManuallyDrop,
  sync::Arc,
  task::{RawWaker, RawWakerVTable, Waker},
};

use futures::Future;
use moonwave_common::*;
use moonwave_resources::*;

pub struct CommandEncoderOutput {
  pub(crate) command_buffer: wgpu::CommandBuffer,
}
impl CommandEncoderOutput {
  pub fn from_raw(buffer: wgpu::CommandBuffer) -> Self {
    Self {
      command_buffer: buffer,
    }
  }
}

pub struct CommandEncoder<'a> {
  encoder: wgpu::CommandEncoder,
  device: &'a wgpu::Device,
}

impl<'a> CommandEncoder<'a> {
  pub fn new(device: &'a wgpu::Device, name: &str) -> Self {
    let encoder =
      device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(name) });

    Self { encoder, device }
  }

  pub fn write_buffer(&mut self, buffer: &ResourceRc<Buffer>, data: &[u8]) {
    optick::event!("CommandEncoder::write_buffer");

    // Create future
    let fut = async {
      let raw_buffer = buffer.get_raw();
      let slice = raw_buffer.slice(0..);
      slice.map_async(wgpu::MapMode::Write).await.unwrap();
      let mut writeable = slice.get_mapped_range_mut();
      writeable.clone_from_slice(data);
      raw_buffer.unmap();
    };

    // Since this is executed multithreaded anyway we simply block the current thread until its ready.
    // When the mapping is not done yet we poll the device that will continue the process on GPU.
    let waker = waker_fn(|| {});
    let mut ctx = std::task::Context::from_waker(&waker);
    let mut box_fut = Box::pin(fut);
    loop {
      match box_fut.as_mut().poll(&mut ctx) {
        std::task::Poll::Ready(output) => return output,
        std::task::Poll::Pending => {
          self.device.poll(wgpu::Maintain::Poll);
        }
      }
    }
  }

  /// Copies one buffer into another
  pub fn copy_buffer_to_buffer(
    &mut self,
    source: &ResourceRc<Buffer>,
    destination: &ResourceRc<Buffer>,
    size: u64,
  ) {
    self.copy_buffer_to_buffer_offseted(source, 0, destination, 0, size)
  }

  /// Copies one buffer into another
  pub fn copy_buffer_to_buffer_offseted(
    &mut self,
    source: &ResourceRc<Buffer>,
    offset_source: u64,
    destination: &ResourceRc<Buffer>,
    offset_destination: u64,
    size: u64,
  ) {
    optick::event!("CommandEncoder::copy_buffer_to_buffer");
    self.encoder.copy_buffer_to_buffer(
      &*source.get_raw(),
      offset_source,
      &*destination.get_raw(),
      offset_destination,
      size,
    )
  }

  /// Creates a new render pass encoder.
  pub fn create_render_pass_encoder(
    &mut self,
    builder: RenderPassCommandEncoderBuilder,
  ) -> RenderPassCommandEncoder {
    RenderPassCommandEncoder {
      builder,
      encoder: &mut self.encoder,
      _commands: Vec::new(),
    }
  }

  /// Stops all recording and builds a new command buffer.
  pub fn finish(self) -> CommandEncoderOutput {
    CommandEncoderOutput {
      command_buffer: self.encoder.finish(),
    }
  }
}

pub struct RenderPassCommandEncoderBuilder {
  name: String,
  outputs: Vec<(ResourceRc<TextureView>, ColorRGB32)>,
  depth: Option<ResourceRc<TextureView>>,
}

impl RenderPassCommandEncoderBuilder {
  pub fn new(name: &str) -> Self {
    Self {
      name: name.to_string(),
      outputs: Vec::new(),
      depth: None,
    }
  }

  pub fn add_color_output(&mut self, view: &ResourceRc<TextureView>, clear: ColorRGB32) {
    self.outputs.push((view.clone(), clear));
  }

  pub fn add_depth(&mut self, view: &ResourceRc<TextureView>) {
    self.depth = Some(view.clone());
  }
}

pub fn get_wgpu_color_rgb(color: ColorRGB32) -> wgpu::Color {
  wgpu::Color {
    r: color.x as f64,
    g: color.y as f64,
    b: color.z as f64,
    a: 1.0,
  }
}

pub enum RenderPassCommand {}

pub struct RenderPassCommandEncoder<'a> {
  builder: RenderPassCommandEncoderBuilder,
  encoder: &'a mut wgpu::CommandEncoder,
  _commands: Vec<RenderPassCommand>,
}

impl<'a> Drop for RenderPassCommandEncoder<'a> {
  fn drop(&mut self) {
    let outputs = self
      .builder
      .outputs
      .iter()
      .map(|output| (output.0.get_raw(), output.1))
      .collect::<Vec<_>>();

    let depth = self.builder.depth.as_ref().map(|output| output.get_raw());

    let _render_pass = self.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
      label: Some(self.builder.name.as_str()),
      color_attachments: &outputs
        .iter()
        .map(|output| wgpu::RenderPassColorAttachmentDescriptor {
          resolve_target: None,
          attachment: &*output.0,
          ops: wgpu::Operations {
            store: true,
            load: wgpu::LoadOp::Clear(get_wgpu_color_rgb(output.1)),
          },
        })
        .collect::<Vec<_>>(),
      depth_stencil_attachment: depth.as_ref().map(|depth| {
        wgpu::RenderPassDepthStencilAttachmentDescriptor {
          attachment: &*depth,
          depth_ops: Some(wgpu::Operations {
            store: true,
            load: wgpu::LoadOp::Clear(0.0),
          }),
          stencil_ops: None,
        }
      }),
    });
  }
}

/*
impl<'a> RenderPassCommandEncoder<'a> {
}
*/

pub fn waker_fn<F: Fn() + Send + Sync + 'static>(f: F) -> Waker {
  let raw = Arc::into_raw(Arc::new(f)) as *const ();
  let vtable = &Helper::<F>::VTABLE;
  unsafe { Waker::from_raw(RawWaker::new(raw, vtable)) }
}

struct Helper<F>(F);

impl<F: Fn() + Send + Sync + 'static> Helper<F> {
  const VTABLE: RawWakerVTable = RawWakerVTable::new(
    Self::clone_waker,
    Self::wake,
    Self::wake_by_ref,
    Self::drop_waker,
  );

  unsafe fn clone_waker(ptr: *const ()) -> RawWaker {
    let arc = ManuallyDrop::new(Arc::from_raw(ptr as *const F));
    std::mem::forget(arc.clone());
    RawWaker::new(ptr, &Self::VTABLE)
  }

  unsafe fn wake(ptr: *const ()) {
    let arc = Arc::from_raw(ptr as *const F);
    (arc)();
  }

  unsafe fn wake_by_ref(ptr: *const ()) {
    let arc = ManuallyDrop::new(Arc::from_raw(ptr as *const F));
    (arc)();
  }

  unsafe fn drop_waker(ptr: *const ()) {
    drop(Arc::from_raw(ptr as *const F));
  }
}
