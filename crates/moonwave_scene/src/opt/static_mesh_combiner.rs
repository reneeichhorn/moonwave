use std::{any::Any, cell::RefCell, marker::PhantomData};

use itertools::Itertools;
use moonwave_common::{
  bytemuck::{cast_slice, Pod},
  *,
};
use moonwave_core::*;
use moonwave_render::RenderPassCommandEncoder;
use moonwave_resources::*;
use parking_lot::{Mutex, MutexGuard};
use rayon::prelude::*;

use crate::{
  memory::{
    SharedAreaBuffer, SharedAreaBufferAllocation, SharedAreaBufferOptions, SharedSimpleBuffer,
    SharedSimpleBufferAllocation,
  },
  Mesh, MeshIndex, MeshVertex, MeshVertexNormal, Transform,
};

thread_local! {
  static STAGING_BELL: RefCell<StagingBelt> = RefCell::new(StagingBelt::new(2 * 1024 * 1024));
}

pub struct StaticMeshCombiner<T: Sized, I: Sized = u16> {
  vertices_per_chunk: usize,
  indices_per_chunk: usize,
  max_vertex_chunks: usize,
  max_index_chunks: usize,
  generations: Mutex<Vec<StaticMeshCombinerGeneration>>,
  _p: PhantomData<(T, I)>,
}

struct StaticMeshCombinerGeneration {
  vertex_manager: Mutex<SharedSimpleBuffer>,
  index_manager: Mutex<SharedAreaBuffer>,
  vertex_buffer: ResourceRc<Buffer>,
  index_buffer: ResourceRc<Buffer>,
  was_full: bool,
}

impl StaticMeshCombinerGeneration {
  pub fn new<T, I>(
    vertices_per_chunk: usize,
    indices_per_chunk: usize,
    max_vertex_chunks: usize,
    max_index_chunks: usize,
  ) -> Self {
    let vertex_manager = SharedSimpleBuffer::new(max_vertex_chunks);
    let index_manager = SharedAreaBuffer::with_options(SharedAreaBufferOptions {
      total_chunks: max_index_chunks,
      cluster_chunk_size: 512,
      max_space_between: 1024,
      ..Default::default()
    });

    let core = Core::get_instance();
    let vertex_buffer = core.create_buffer(
      (vertices_per_chunk * max_vertex_chunks * std::mem::size_of::<T>()) as u64,
      false,
      BufferUsage::VERTEX | BufferUsage::COPY_DST,
      Some("static_mesh_combiner_vbuffer"),
    );
    let index_buffer = core.create_buffer(
      (indices_per_chunk * max_index_chunks * std::mem::size_of::<I>()) as u64,
      false,
      BufferUsage::INDEX | BufferUsage::COPY_DST,
      Some("static_mesh_combiner_ibuffer"),
    );

    Self {
      vertex_manager: Mutex::new(vertex_manager),
      index_manager: Mutex::new(index_manager),
      vertex_buffer,
      index_buffer,
      was_full: false,
    }
  }
}

pub trait GenericStaticMeshCombiner: Any + Send + Sync + 'static {
  fn merged_draw(&self, entries: &[StaticMeshCombinerEntry], pass: &mut RenderPassCommandEncoder);
  fn as_any(&self) -> &dyn Any;
}

impl<T: Send + Sync + 'static, I: MeshIndex + Send + Sync + 'static> GenericStaticMeshCombiner
  for StaticMeshCombiner<T, I>
{
  fn as_any(&self) -> &dyn Any {
    self
  }

  fn merged_draw(&self, entries: &[StaticMeshCombinerEntry], pass: &mut RenderPassCommandEncoder) {
    if entries.is_empty() {
      return;
    }

    // Sort by index start position.
    let mut sorted_entries = entries.iter().collect_vec();
    sorted_entries.sort_unstable_by_key(|e| e.ib.chunk_start);

    // Merge calls
    /*
    let mut current_start = sorted_entries[0].ib.chunk_start * self.indices_per_chunk;
    let mut current_length = sorted_entries[0].indices;
    let mut undrawn = Some(sorted_entries[0]);
    */

    let mut prev_generation = usize::MAX;
    for entry in sorted_entries.into_iter() {
      if entry.generation != prev_generation {
        let generations = self.generations.lock();
        let generation = generations.get(entry.generation).unwrap();
        pass.set_vertex_buffer(generation.vertex_buffer.clone());
        pass.set_index_buffer(generation.index_buffer.clone(), I::get_format());
        prev_generation = entry.generation;
      }
      let start = entry.ib.chunk_start * self.indices_per_chunk;
      pass.render_indexed(start as u32..(start + entry.indices) as u32);
    }

    /*
    for entry in sorted_entries.into_iter().skip(1) {
      let next_start = entry.ib.chunk_start * self.indices_per_chunk;

      // Check if it is a follow up
      if next_start == (current_start + current_length) && entry.generation == prev_generation {
        undrawn = Some(entry);
        current_length += entry.indices;
        continue;
      }

      // Check if we need to also have a generation switch.
      if entry.generation != prev_generation {
        let generations = self.generations.lock();
        let generation = generations.get(entry.generation).unwrap();
        pass.set_vertex_buffer(generation.vertex_buffer.clone());
        pass.set_index_buffer(generation.index_buffer.clone(), I::get_format());
        prev_generation = entry.generation;
      }

      // Not a follow up therefore we need to have a render call here.
      undrawn = None;
      pass.render_indexed(current_start as u32..(current_start + current_length) as u32);
      current_start = next_start;
      current_length = entry.indices;
    }

    // End up with a render call.
    if let Some(undrawn) = undrawn {
      if undrawn.generation != prev_generation {
        let generations = self.generations.lock();
        let generation = generations.get(undrawn.generation).unwrap();
        pass.set_vertex_buffer(generation.vertex_buffer.clone());
        pass.set_index_buffer(generation.index_buffer.clone(), I::get_format());
      }
      pass.render_indexed(current_start as u32..(current_start + current_length) as u32);
    }
    */
  }
}

impl<
    T: Sized + Pod + MeshVertex + MeshVertexNormal + Sync + Send + 'static,
    I: Sized + Pod + MeshIndex + Sync + Send + 'static,
  > StaticMeshCombiner<T, I>
{
  pub fn new(
    vertices_per_chunk: usize,
    indices_per_chunk: usize,
    max_vertex_chunks: usize,
    max_index_chunks: usize,
  ) -> Self {
    Self {
      vertices_per_chunk,
      indices_per_chunk,
      max_vertex_chunks,
      max_index_chunks,
      generations: Mutex::new(vec![StaticMeshCombinerGeneration::new::<T, I>(
        vertices_per_chunk,
        indices_per_chunk,
        max_vertex_chunks,
        max_index_chunks,
      )]),
      _p: PhantomData {},
    }
  }

  pub fn remove(&self, entry: StaticMeshCombinerEntry) {
    let mut generations = self.generations.lock();
    let generation = generations.get_mut(entry.generation).unwrap();

    // Remove vertex allocations.
    {
      let mut manager = generation.vertex_manager.lock();
      for v in entry.vb {
        manager.free(v);
      }
    }

    // Remove index allocations.
    {
      let mut manager = generation.index_manager.lock();
      manager.free(entry.ib);
    }

    // Might no longer be full
    generation.was_full = false;
  }

  pub fn insert(
    &self,
    mesh: &Mesh<T, I>,
    transform: &Transform,
  ) -> Option<StaticMeshCombinerEntry> {
    // Try to find free slot within any generation created so far.
    let mut generations = self.generations.lock();
    for generation in 0..generations.len() {
      let entry = self.insert_into_generation(mesh, transform, &mut generations, generation);
      if entry.is_some() {
        return entry;
      }
    }

    // Create a new generation
    let new_generation = generations.len();
    generations.push(StaticMeshCombinerGeneration::new::<T, I>(
      self.vertices_per_chunk,
      self.indices_per_chunk,
      self.max_vertex_chunks,
      self.max_index_chunks,
    ));
    self.insert_into_generation(mesh, transform, &mut generations, new_generation)
  }

  fn insert_into_generation(
    &self,
    mesh: &Mesh<T, I>,
    transform: &Transform,
    generations: &mut MutexGuard<'_, Vec<StaticMeshCombinerGeneration>>,
    generation_index: usize,
  ) -> Option<StaticMeshCombinerEntry> {
    let generation = generations.get_mut(generation_index).unwrap();
    if generation.was_full {
      return None;
    }

    // Get mesh stats.
    let total_indices = mesh.len_indices();
    let total_vertices = mesh.len_vertices();
    let chunks_indices = (total_indices as f32 / self.indices_per_chunk as f32).ceil() as usize;
    let chunks_vertices = (total_vertices as f32 / self.vertices_per_chunk as f32).ceil() as usize;

    // Get free space in vertex buffer.
    let vb = {
      let mut manager = generation.vertex_manager.lock();
      let vb = (0..chunks_vertices)
        .filter_map(|_| manager.alloc())
        .collect_vec();

      // If not enough allocated worked free all and return None as an OOM indication.
      if vb.len() != chunks_vertices {
        for v in vb {
          manager.free(v);
        }
        generation.was_full = true;
        return None;
      }

      vb
    };

    // Get free space in index buffer
    let transform_inner = transform.get();
    let ib = {
      let mut manager = generation.index_manager.lock();
      let ib = manager.alloc(chunks_indices, transform_inner.position);
      // When no indices are available free all reserved in vertex buffer and indicate OOM.
      if ib.is_none() {
        let mut vmanager = generation.vertex_manager.lock();
        for v in vb {
          vmanager.free(v);
        }
        generation.was_full = true;
        return None;
      }
      ib.unwrap()
    };

    // Build vertex buffer with preapplied transform.
    let transform_matrix = transform.calculate_transform_matrix();
    let normal_matrix = transform_matrix;

    let vertices = mesh
      .iter_vertices()
      .map(|vertex| {
        /*
        let position = transform_matrix.transform_vector(*vertex.get_position());
        let normal = normal_matrix.transform_vector(*vertex.get_normal());
        let tangent = normal_matrix.transform_vector(*vertex.get_tangent());
        let bitangent = normal_matrix.transform_vector(*vertex.get_bitangent());

        let mut new_vertex = *vertex;
        *new_vertex.get_position_mut() = position;
        *new_vertex.get_normal_mut() = normal;
        *new_vertex.get_tangent_mut() = tangent;
        *new_vertex.get_bitangent_mut() = bitangent;

        new_vertex
        */
        let position = transform_matrix
          * Vector4::new(
            vertex.get_position().x,
            vertex.get_position().y,
            vertex.get_position().z,
            1.0,
          );
        let normal = normal_matrix
          * Vector4::new(
            vertex.get_normal().x,
            vertex.get_normal().y,
            vertex.get_normal().z,
            1.0,
          );
        let tangent = normal_matrix
          * Vector4::new(
            vertex.get_tangent().x,
            vertex.get_tangent().y,
            vertex.get_tangent().z,
            1.0,
          );
        let bitangent = normal_matrix
          * Vector4::new(
            vertex.get_bitangent().x,
            vertex.get_bitangent().y,
            vertex.get_bitangent().z,
            1.0,
          );

        let mut new_vertex = *vertex;
        *new_vertex.get_position_mut() = position.xyz().div_element_wise(position.w);
        /*
         *new_vertex.get_normal_mut() = normal.xyz().div_element_wise(normal.w);
         *new_vertex.get_tangent_mut() = tangent.xyz().div_element_wise(tangent.w);
         *new_vertex.get_bitangent_mut() = bitangent.xyz().div_element_wise(bitangent.w);
         */

        new_vertex
      })
      .collect::<Vec<_>>();

    // Place vertices into buffer.
    let vertices_data = cast_slice(&vertices);
    let vertex_buffer = generation.vertex_buffer.clone();
    let chunk_slices = vertices_data.par_chunks(self.vertices_per_chunk * std::mem::size_of::<T>());

    chunk_slices
      .into_par_iter()
      .zip(vb.par_iter())
      .for_each(move |(chunk, alloc)| {
        STAGING_BELL.with(|belt| {
          let mut belt = belt.borrow_mut();
          let mem_address = alloc.index * self.vertices_per_chunk * std::mem::size_of::<T>();
          belt.write_immediate(&vertex_buffer, mem_address as u64, chunk);
        });
      });

    // Rebuild indices for mesh with new offsets.
    let indices = mesh
      .iter_indices()
      .map(|index| {
        let chunk_index = index.as_usize() / self.vertices_per_chunk;
        index.with_offset(vb[chunk_index].index * self.vertices_per_chunk)
      })
      .collect_vec();

    // Place indices into buffer.
    let indices_raw = cast_slice(&indices);
    STAGING_BELL.with(|belt| {
      let mut belt = belt.borrow_mut();
      let mem_address = ib.chunk_start * self.indices_per_chunk * std::mem::size_of::<I>();
      belt.write_immediate(&generation.index_buffer, mem_address as u64, indices_raw);
    });

    Some(StaticMeshCombinerEntry {
      vb,
      ib,
      indices: mesh.len_indices(),
      generation: generation_index,
    })
  }
}

#[derive(Clone, Debug)]
pub struct StaticMeshCombinerEntry {
  vb: Vec<SharedSimpleBufferAllocation>,
  ib: SharedAreaBufferAllocation,
  indices: usize,
  generation: usize,
}
