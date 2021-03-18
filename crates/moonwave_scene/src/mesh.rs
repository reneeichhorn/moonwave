use moonwave_common::{
  bytemuck::{cast_slice, Pod, Zeroable},
  Vector3,
};
use moonwave_core::Core;
use moonwave_resources::{Buffer, BufferUsage, IndexFormat, ResourceRc};
use moonwave_shader::VertexStruct;

pub struct Mesh<T: MeshVertex, I: MeshIndex> {
  indices: Vec<I>,
  vertices: Vec<T>,
}

impl<T: MeshVertex, I: MeshIndex> Mesh<T, I> {
  pub fn new() -> Self {
    Self {
      vertices: Vec::new(),
      indices: Vec::new(),
    }
  }

  pub fn new_merged<'a>(groups: impl Iterator<Item = &'a Self>) -> Self {
    let mut merged = Mesh::new();
    let mut offset = 0;

    for mesh in groups {
      merged.vertices.extend(&mesh.vertices);
      merged
        .indices
        .extend(mesh.indices.iter().map(|i| i.with_offset(offset)));

      offset += mesh.vertices.len();
    }

    merged
  }

  pub fn push_vertex(&mut self, vertex: T) {
    self.vertices.push(vertex);
  }

  pub fn push_index(&mut self, index: I) {
    self.indices.push(index);
  }

  pub fn len_indices(&self) -> usize {
    self.indices.len()
  }

  pub fn build_vertex_buffer(&self) -> ResourceRc<Buffer> {
    // Build raw
    let raw = cast_slice(&self.vertices);
    let raw_boxed = Box::from(raw);

    // Build buffer.
    Core::get_instance().create_inited_buffer(raw_boxed, BufferUsage::VERTEX, None)
  }

  pub fn build_index_buffer(&self) -> ResourceRc<Buffer> {
    // Build raw
    let raw = cast_slice(&self.indices);
    let raw_boxed = Box::from(raw);

    // Build buffer.
    Core::get_instance().create_inited_buffer(raw_boxed, BufferUsage::INDEX, None)
  }
}

pub trait MeshVertex: Zeroable + Pod + VertexStruct {
  fn get_position(&self) -> &Vector3<f32>;
  fn get_position_mut(&mut self) -> &mut Vector3<f32>;
}

pub trait MeshIndex: Pod {
  fn with_offset(self, offset: usize) -> Self;
  fn get_format() -> IndexFormat;
}
impl MeshIndex for u16 {
  fn with_offset(self, offset: usize) -> Self {
    self + offset as u16
  }
  fn get_format() -> IndexFormat {
    IndexFormat::Uint16
  }
}
impl MeshIndex for u32 {
  fn with_offset(self, offset: usize) -> Self {
    self + offset as u32
  }
  fn get_format() -> IndexFormat {
    IndexFormat::Uint32
  }
}
