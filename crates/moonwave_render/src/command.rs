use std::{
  mem::ManuallyDrop,
  ops::Range,
  sync::Arc,
  task::{RawWaker, RawWakerVTable, Waker},
};

use futures::Future;
use moonwave_common::*;
use moonwave_resources::*;
use wgpu::util::RenderEncoder;

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
      {
        let slice = raw_buffer.slice(0..data.len() as u64);
        slice.map_async(wgpu::MapMode::Write).await.unwrap();
        let mut writeable = slice.get_mapped_range_mut();
        writeable.clone_from_slice(data);
      }
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
      commands: Vec::new(),
    }
  }

  /// Stops all recording and builds a new command buffer.
  pub fn finish(self) -> CommandEncoderOutput {
    CommandEncoderOutput {
      command_buffer: self.encoder.finish(),
    }
  }
}

#[derive(Clone)]
pub struct RenderPassCommandEncoderBuilder {
  name: String,
  outputs: Vec<(ResourceRc<TextureView>, ColorRGBA32)>,
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

  pub fn add_color_output(&mut self, view: &ResourceRc<TextureView>, clear: ColorRGBA32) {
    self.outputs.push((view.clone(), clear));
  }

  pub fn add_depth(&mut self, view: &ResourceRc<TextureView>) {
    self.depth = Some(view.clone());
  }
}

pub fn get_wgpu_color_rgb(color: ColorRGBA32) -> wgpu::Color {
  wgpu::Color {
    r: color.x as f64,
    g: color.y as f64,
    b: color.z as f64,
    a: color.w as f64,
  }
}

enum RenderPassCommand {
  SetRenderPipeline(ResourceRc<RenderPipeline>),
  SetVertexBuffer(ResourceRc<Buffer>),
  SetIndexBuffer(IndexFormat, ResourceRc<Buffer>),
  SetBindGroup(u32, ResourceRc<BindGroup>),
  RenderIndexed(Range<u32>),
}

pub struct RenderPassCommandEncoder<'a> {
  builder: RenderPassCommandEncoderBuilder,
  encoder: &'a mut wgpu::CommandEncoder,
  commands: Vec<RenderPassCommand>,
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

    // Create render pass.
    let mut rp = self.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
            load: wgpu::LoadOp::Clear(1.0),
          }),
          stencil_ops: None,
        }
      }),
    });

    // Execute commands.
    for command in self.commands.iter() {
      match command {
        RenderPassCommand::SetRenderPipeline(pipeline) => rp.set_pipeline(pipeline.get_raw()),
        RenderPassCommand::SetBindGroup(binding, bind) => {
          rp.set_bind_group(*binding, bind.get_raw(), &[])
        }
        RenderPassCommand::SetVertexBuffer(buffer) => {
          rp.set_vertex_buffer(0, buffer.get_raw().slice(0..))
        }
        RenderPassCommand::SetIndexBuffer(format, buffer) => {
          rp.set_index_buffer(buffer.get_raw().slice(0..), *format)
        }
        RenderPassCommand::RenderIndexed(range) => rp.draw_indexed(range.clone(), 0, 0..1),
        _ => {}
      }
    }
  }
}

impl<'a> RenderPassCommandEncoder<'a> {
  pub fn set_pipeline(&mut self, pipeline: ResourceRc<RenderPipeline>) {
    self
      .commands
      .push(RenderPassCommand::SetRenderPipeline(pipeline));
  }

  pub fn set_vertex_buffer(&mut self, buffer: ResourceRc<Buffer>) {
    self
      .commands
      .push(RenderPassCommand::SetVertexBuffer(buffer));
  }

  pub fn set_index_buffer(&mut self, buffer: ResourceRc<Buffer>, format: IndexFormat) {
    self
      .commands
      .push(RenderPassCommand::SetIndexBuffer(format, buffer));
  }

  pub fn set_bind_group(&mut self, binding: u32, bind_group: ResourceRc<BindGroup>) {
    self
      .commands
      .push(RenderPassCommand::SetBindGroup(binding, bind_group));
  }

  pub fn render_indexed(&mut self, range: Range<u32>) {
    self.commands.push(RenderPassCommand::RenderIndexed(range));
  }
}

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
