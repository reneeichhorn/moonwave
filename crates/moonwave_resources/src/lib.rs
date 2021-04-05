#![allow(clippy::new_without_default)]

use std::marker::PhantomData;
use std::sync::Arc;

pub use wgpu::{IndexFormat, TextureFormat, TextureUsage};

struct ResourceLife {
  original: Resource,
}

impl Drop for ResourceLife {
  fn drop(&mut self) {
    optick::event!("ResourceStorage::release_resource");
  }
}

pub struct ResourceRc<T> {
  life: Arc<ResourceLife>,
  _ty: PhantomData<T>,
}

impl<T> Clone for ResourceRc<T> {
  fn clone(&self) -> ResourceRc<T> {
    ResourceRc {
      _ty: PhantomData,
      life: self.life.clone(),
    }
  }
}

pub trait IntoResource {
  type ProxyType;
  fn into(self) -> Resource;
}

pub enum Resource {
  Texture(wgpu::Texture),
  TextureView(wgpu::TextureView),
  Sampler(wgpu::Sampler),
  Buffer(wgpu::Buffer),
  Shader(wgpu::ShaderModule),
  BindGroupLayout(wgpu::BindGroupLayout),
  PipelineLayout(wgpu::PipelineLayout),
  BindGroup(wgpu::BindGroup),
  RenderPipeline(wgpu::RenderPipeline),
}

pub struct ResourceStorage;

impl ResourceStorage {
  pub fn new() -> Self {
    Self {}
  }

  pub fn create_proxy<T: IntoResource>(&self, resource: T) -> ResourceRc<T::ProxyType> {
    ResourceRc {
      life: Arc::new(ResourceLife {
        original: resource.into(),
      }),
      _ty: PhantomData,
    }
  }
}

// Resource types
macro_rules! make_into_resource {
  ($proxy:ident, $org:ident) => {
    pub struct $proxy;
    impl IntoResource for wgpu::$org {
      type ProxyType = $proxy;
      fn into(self) -> Resource {
        Resource::$proxy(self)
      }
    }

    impl ResourceRc<$proxy> {
      pub fn get_raw(&self) -> &wgpu::$org {
        if let Resource::$proxy(res) = &self.life.original {
          &res
        } else {
          panic!("Unexpected resource type at slot") // won't happen due to phantom type safety.
        }
      }
    }
  };
}
make_into_resource!(Buffer, Buffer);
make_into_resource!(Sampler, Sampler);
make_into_resource!(Texture, Texture);
make_into_resource!(TextureView, TextureView);
make_into_resource!(Shader, ShaderModule);
make_into_resource!(PipelineLayout, PipelineLayout);
make_into_resource!(BindGroupLayout, BindGroupLayout);
make_into_resource!(BindGroup, BindGroup);
make_into_resource!(RenderPipeline, RenderPipeline);

// Definition structures
#[derive(Clone, Copy, Debug)]
pub enum VertexAttributeFormat {
  Float4,
  Float3,
  Float2,
  Float,
  UInt4,
  UInt3,
  UInt2,
  UInt,
}

impl VertexAttributeFormat {
  pub fn to_wgpu(&self) -> wgpu::VertexFormat {
    match self {
      VertexAttributeFormat::Float4 => wgpu::VertexFormat::Float4,
      VertexAttributeFormat::Float3 => wgpu::VertexFormat::Float3,
      VertexAttributeFormat::Float2 => wgpu::VertexFormat::Float2,
      VertexAttributeFormat::Float => wgpu::VertexFormat::Float,
      VertexAttributeFormat::UInt4 => wgpu::VertexFormat::Uint4,
      VertexAttributeFormat::UInt3 => wgpu::VertexFormat::Uint3,
      VertexAttributeFormat::UInt2 => wgpu::VertexFormat::Uint2,
      VertexAttributeFormat::UInt => wgpu::VertexFormat::Uint,
    }
  }
}

#[derive(Clone)]
pub struct VertexAttribute {
  pub name: String,
  pub offset: u64,
  pub format: VertexAttributeFormat,
  pub location: usize,
}

#[derive(Clone)]
pub struct VertexBuffer {
  pub stride: u64,
  pub attributes: Vec<VertexAttribute>,
}

// Wrapper for wgpu buffer usage bit flags.
bitflags::bitflags! {
    /// Different ways that you can use a buffer.
    ///
    /// The usages determine what kind of memory the buffer is allocated from and what
    /// actions the buffer can partake in.
    pub struct BufferUsage: u32 {
        /// Allow a buffer to be mapped for reading using [`Buffer::map_async`] + [`Buffer::get_mapped_range`].
        /// This does not include creating a buffer with [`BufferDescriptor::mapped_at_creation`] set.
        ///
        /// If [`Features::MAPPABLE_PRIMARY_BUFFERS`] isn't enabled, the only other usage a buffer
        /// may have is COPY_DST.
        const MAP_READ = 1;
        /// Allow a buffer to be mapped for writing using [`Buffer::map_async`] + [`Buffer::get_mapped_range_mut`].
        /// This does not include creating a buffer with `mapped_at_creation` set.
        ///
        /// If [`Features::MAPPABLE_PRIMARY_BUFFERS`] feature isn't enabled, the only other usage a buffer
        /// may have is COPY_SRC.
        const MAP_WRITE = 2;
        /// Allow a buffer to be the source buffer for a [`CommandEncoder::copy_buffer_to_buffer`] or [`CommandEncoder::copy_buffer_to_texture`]
        /// operation.
        const COPY_SRC = 4;
        /// Allow a buffer to be the destination buffer for a [`CommandEncoder::copy_buffer_to_buffer`], [`CommandEncoder::copy_texture_to_buffer`],
        /// or [`Queue::write_buffer`] operation.
        const COPY_DST = 8;
        /// Allow a buffer to be the index buffer in a draw operation.
        const INDEX = 16;
        /// Allow a buffer to be the vertex buffer in a draw operation.
        const VERTEX = 32;
        /// Allow a buffer to be a [`BufferBindingType::Uniform`] inside a bind group.
        const UNIFORM = 64;
        /// Allow a buffer to be a [`BufferBindingType::Storage`] inside a bind group.
        const STORAGE = 128;
        /// Allow a buffer to be the indirect buffer in an indirect draw call.
        const INDIRECT = 256;
    }
}

pub struct BindGroupLayoutDescriptor {
  pub entries: Vec<BindGroupLayoutEntry>,
}

pub struct BindGroupLayoutEntry {
  pub binding: u32,
  pub ty: BindGroupLayoutEntryType,
}

pub enum BindGroupLayoutEntryType {
  UniformBuffer,
  Sampler,
  SingleTexture,
}

impl BindGroupLayoutDescriptor {
  pub fn new() -> Self {
    Self {
      entries: Vec::new(),
    }
  }

  pub fn add_entry(mut self, binding: u32, ty: BindGroupLayoutEntryType) -> Self {
    self.entries.push(BindGroupLayoutEntry { binding, ty });
    self
  }
}

pub struct PipelineLayoutDescriptor {
  pub bindings: Vec<ResourceRc<BindGroupLayout>>,
}
impl PipelineLayoutDescriptor {
  pub fn new() -> Self {
    Self {
      bindings: Vec::new(),
    }
  }

  pub fn add_binding(mut self, resource: ResourceRc<BindGroupLayout>) -> Self {
    self.bindings.push(resource);
    self
  }
}

pub struct BindGroupDescriptor {
  pub layout: ResourceRc<BindGroupLayout>,
  pub entries: Vec<(u32, BindGroupEntry)>,
}

impl BindGroupDescriptor {
  pub fn new(layout: ResourceRc<BindGroupLayout>) -> Self {
    Self {
      layout,
      entries: Vec::new(),
    }
  }

  pub fn add_buffer_binding(mut self, binding: u32, buffer: ResourceRc<Buffer>) -> Self {
    self.entries.push((binding, BindGroupEntry::Buffer(buffer)));
    self
  }

  pub fn add_texture_binding(mut self, binding: u32, texture: ResourceRc<TextureView>) -> Self {
    self
      .entries
      .push((binding, BindGroupEntry::Texture(texture)));
    self
  }

  pub fn add_sampler_binding(mut self, binding: u32, sampler: ResourceRc<Sampler>) -> Self {
    self
      .entries
      .push((binding, BindGroupEntry::Sampler(sampler)));
    self
  }
}

pub enum BindGroupEntry {
  Buffer(ResourceRc<Buffer>),
  Texture(ResourceRc<TextureView>),
  Sampler(ResourceRc<Sampler>),
}

pub struct RenderPipelineDescriptor {
  pub layout: ResourceRc<PipelineLayout>,
  pub vertex_shader: ResourceRc<Shader>,
  pub vertex_desc: Option<VertexBuffer>,
  pub fragment_shader: ResourceRc<Shader>,
  pub outputs: Vec<RenderPipelineOutput>,
  pub depth: Option<TextureFormat>,
}

pub struct RenderPipelineOutput {
  pub format: TextureFormat,
}

impl RenderPipelineDescriptor {
  pub fn new(
    layout: ResourceRc<PipelineLayout>,
    vertex_desc: VertexBuffer,
    vertex_shader: ResourceRc<Shader>,
    fragment_shader: ResourceRc<Shader>,
  ) -> Self {
    Self {
      layout,
      vertex_desc: Some(vertex_desc),
      vertex_shader,
      fragment_shader,
      depth: None,
      outputs: Vec::new(),
    }
  }

  pub fn new_without_vertices(
    layout: ResourceRc<PipelineLayout>,
    vertex_shader: ResourceRc<Shader>,
    fragment_shader: ResourceRc<Shader>,
  ) -> Self {
    Self {
      layout,
      vertex_desc: None,
      vertex_shader,
      fragment_shader,
      depth: None,
      outputs: Vec::new(),
    }
  }

  pub fn add_color_output(mut self, format: TextureFormat) -> Self {
    self.outputs.push(RenderPipelineOutput { format });
    self
  }

  pub fn add_depth(mut self, format: TextureFormat) -> Self {
    self.depth = Some(format);
    self
  }
}

#[derive(Clone)]
pub struct SampledTexture {
  pub texture: ResourceRc<Texture>,
  pub view: ResourceRc<TextureView>,
  pub sampler: ResourceRc<Sampler>,
  pub bind_group: ResourceRc<BindGroup>,
}
