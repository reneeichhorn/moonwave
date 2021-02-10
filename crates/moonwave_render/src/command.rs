use std::ops::RangeBounds;

use moonwave_common::*;

pub struct CommandEncoder {
  encoder: wgpu::CommandEncoder,
}

impl CommandEncoder {
  pub fn new(device: &wgpu::Device, name: &str) -> Self {
    let encoder =
      device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(name) });
    Self { encoder }
  }

  /// Copies one buffer into another
  pub fn copy_buffer_to_buffer<T: BufferRef>(&mut self, source: &T, destination: &T, size: u64) {
    self.copy_buffer_to_buffer_offseted(source, 0, destination, 0, size)
  }

  /// Copies one buffer into another
  pub fn copy_buffer_to_buffer_offseted<T: BufferRef>(
    &mut self,
    source: &T,
    offset_source: u64,
    destination: &T,
    offset_destination: u64,
    size: u64,
  ) {
    self.encoder.copy_buffer_to_buffer(
      source.get_buffer(),
      offset_source,
      destination.get_buffer(),
      offset_destination,
      size,
    )
  }

  pub fn create_render_pass_encoder<'a>(
    &'a mut self,
    builder: &'a RenderPassCommandEncoderBuilder,
  ) -> RenderPassCommandEncoder<'a> {
    let render_pass = self.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
      label: Some(builder.name.as_str()),
      color_attachments: &builder
        .outputs
        .iter()
        .map(|output| wgpu::RenderPassColorAttachmentDescriptor {
          resolve_target: None,
          attachment: output.0.get_texture_view(),
          ops: wgpu::Operations {
            store: true,
            load: wgpu::LoadOp::Clear(get_wgpu_color_rgb(output.1)),
          },
        })
        .collect::<Vec<_>>(),
      depth_stencil_attachment: builder.depth.as_ref().map(|depth| {
        wgpu::RenderPassDepthStencilAttachmentDescriptor {
          attachment: depth.get_texture_view(),
          depth_ops: Some(wgpu::Operations {
            store: true,
            load: wgpu::LoadOp::Clear(0.0),
          }),
          stencil_ops: None,
        }
      }),
    });

    RenderPassCommandEncoder { render_pass }
  }
}

pub struct RenderPassCommandEncoderBuilder {
  name: String,
  outputs: Vec<(Box<dyn TextureViewRef>, ColorRGB32)>,
  depth: Option<Box<dyn TextureViewRef>>,
}

impl RenderPassCommandEncoderBuilder {
  pub fn new(name: &str) -> Self {
    Self {
      name: name.to_string(),
      outputs: Vec::new(),
      depth: None,
    }
  }

  pub fn add_color_output<T: TextureViewRef>(&mut self, view: &T, clear: ColorRGB32) {
    self.outputs.push((view.get_dyn(), clear));
  }

  pub fn add_depth<T: TextureViewRef>(&mut self, view: &T) {
    self.depth = Some(view.get_dyn());
  }
}

pub trait BufferRef {
  fn get_buffer(&self) -> &wgpu::Buffer;
}

pub trait TextureViewRef: 'static {
  fn get_texture_view(&self) -> &wgpu::TextureView;
  fn get_dyn(&self) -> Box<dyn TextureViewRef>;
}

pub fn get_wgpu_color_rgb(color: ColorRGB32) -> wgpu::Color {
  wgpu::Color {
    r: color.x as f64,
    g: color.y as f64,
    b: color.z as f64,
    a: 1.0,
  }
}

pub struct RenderPassCommandEncoder<'a> {
  render_pass: wgpu::RenderPass<'a>,
}

impl<'a> RenderPassCommandEncoder<'a> {
  pub fn set_vertex_buffer<T: BufferRef, B: RangeBounds<u64>>(
    &mut self,
    slot: u32,
    buffer: &'a T,
    bounds: B,
  ) {
    self
      .render_pass
      .set_vertex_buffer(slot, buffer.get_buffer().slice(bounds))
  }
}
