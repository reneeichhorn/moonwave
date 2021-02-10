use std::marker::PhantomData;
use std::sync::{Arc, RwLock};

use slotmap::{DefaultKey, SlotMap};

struct ResourceLife {
  holder: Arc<RwLock<SlotMap<DefaultKey, Resource>>>,
  key: DefaultKey,
}

impl Drop for ResourceLife {
  fn drop(&mut self) {
    let mut holder = self.holder.write().unwrap();
    holder.remove(self.key);
  }
}

#[derive(Clone)]
pub struct ResourceRc<T> {
  life: Arc<ResourceLife>,
  _ty: PhantomData<T>,
}

pub trait IntoResource {
  type ProxyType;
  fn into(self) -> Resource;
}

pub enum Resource {
  Buffer(wgpu::Buffer),
  Shader(wgpu::ShaderModule),
}

pub struct ResourceStorage {
  resource_slots: Arc<RwLock<SlotMap<DefaultKey, Resource>>>,
}

impl ResourceStorage {
  pub fn new() -> Self {
    Self {
      resource_slots: Arc::new(RwLock::new(SlotMap::with_capacity(1024))),
    }
  }

  pub fn create_proxy<T: IntoResource>(&self, resource: T) -> ResourceRc<T::ProxyType> {
    let mut resource_slots = self.resource_slots.write().unwrap();
    let key = resource_slots.insert(resource.into());
    ResourceRc {
      life: Arc::new(ResourceLife {
        key,
        holder: self.resource_slots.clone(),
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
  };
}
make_into_resource!(Buffer, Buffer);
make_into_resource!(Shader, ShaderModule);

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
pub struct VertexAttribute {
  pub name: String,
  pub offset: u64,
  pub format: VertexAttributeFormat,
  pub location: usize,
}

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
