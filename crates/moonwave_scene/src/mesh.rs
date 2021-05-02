use moonwave_common::{
  bytemuck::{cast_slice, Pod, Zeroable},
  InnerSpace, Vector2, Vector3,
};
use moonwave_core::{Core, Itertools};
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

  pub fn iter_vertices(&self) -> impl Iterator<Item = &T> + '_ {
    self.vertices.iter()
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

impl<T: MeshVertexNormal + MeshVertexUV, I: MeshIndex> Mesh<T, I> {
  pub fn build_normal_tangent_bitangent(
    &mut self,
    calc_normal: bool,
    calc_tangent: bool,
    calc_bitangent: bool,
  ) {
    // Apply normals on a face to face base.
    for (i1, i2, i3) in self.indices.iter().tuples() {
      // Access vertices per face.
      let v1 = &self.vertices[i1.as_usize()];
      let v2 = &self.vertices[i2.as_usize()];
      let v3 = &self.vertices[i3.as_usize()];
      let is = [i1.as_usize(), i2.as_usize(), i3.as_usize()];

      // Build position deltas
      let delta_pos_1_2 = v1.get_position() - v2.get_position();
      let delta_pos_3_1 = v3.get_position() - v1.get_position();

      // Build normal if wanted.
      let normal = if calc_normal {
        Some(delta_pos_1_2.cross(delta_pos_3_1))
      } else {
        None
      };

      // Build tangent and bitangent
      let (tangent, bitangent) = if calc_bitangent || calc_tangent {
        let delta_pos_2_1 = v2.get_position() - v1.get_position();
        let delta_uv_2_1 = v2.get_uv() - v1.get_uv();
        let delta_uv_3_1 = v3.get_uv() - v1.get_uv();

        let r = 1.0 / (delta_uv_2_1.x * delta_uv_3_1.y - delta_uv_2_1.y * delta_uv_3_1.x);

        // Build tangent.
        let tangent = if calc_tangent {
          Some((delta_pos_2_1 * delta_uv_3_1.y - delta_pos_3_1 * delta_uv_2_1.y) * r)
        } else {
          None
        };

        // Build bitangent.
        let bitangent = if calc_bitangent {
          Some((delta_pos_3_1 * delta_uv_2_1.x - delta_pos_2_1 * delta_uv_3_1.x) * r)
        } else {
          None
        };

        (tangent, bitangent)
      } else {
        (None, None)
      };

      // Append
      for i in is.iter() {
        if let Some(normal) = normal {
          *self.vertices[*i].get_normal_mut() += normal;
        }
        if let Some(tangent) = tangent {
          *self.vertices[*i].get_tangent_mut() += tangent;
        }
        if let Some(bitangent) = bitangent {
          *self.vertices[*i].get_bitangent_mut() += bitangent;
        }
      }
    }

    // Normalize caluclated values for each vertex
    for vertex in &mut self.vertices {
      if calc_normal {
        let normal = vertex.get_normal_mut();
        *normal = normal.normalize();
      }
      if calc_tangent {
        let tangent = vertex.get_tangent_mut();
        *tangent = tangent.normalize();
      }
      if calc_bitangent {
        let bitangent = vertex.get_bitangent_mut();
        *bitangent = bitangent.normalize();
      }
    }
  }
}

pub trait MeshVertex: Zeroable + Pod + VertexStruct {
  fn get_position(&self) -> &Vector3<f32>;
  fn get_position_mut(&mut self) -> &mut Vector3<f32>;
}

pub trait MeshVertexUV: MeshVertex {
  fn get_uv(&self) -> &Vector2<f32>;
  fn get_uv_mut(&mut self) -> &mut Vector2<f32>;
}

pub trait MeshVertexNormal: MeshVertex {
  fn get_normal(&self) -> &Vector3<f32>;
  fn get_normal_mut(&mut self) -> &mut Vector3<f32>;

  fn get_tangent(&self) -> &Vector3<f32>;
  fn get_tangent_mut(&mut self) -> &mut Vector3<f32>;

  fn get_bitangent(&self) -> &Vector3<f32>;
  fn get_bitangent_mut(&mut self) -> &mut Vector3<f32>;
}

pub trait MeshIndex: Pod {
  fn with_offset(self, offset: usize) -> Self;
  fn as_usize(self) -> usize;
  fn get_format() -> IndexFormat;
}
impl MeshIndex for u16 {
  fn as_usize(self) -> usize {
    self as usize
  }
  fn with_offset(self, offset: usize) -> Self {
    self + offset as u16
  }
  fn get_format() -> IndexFormat {
    IndexFormat::Uint16
  }
}
impl MeshIndex for u32 {
  fn as_usize(self) -> usize {
    self as usize
  }
  fn with_offset(self, offset: usize) -> Self {
    self + offset as u32
  }
  fn get_format() -> IndexFormat {
    IndexFormat::Uint32
  }
}
