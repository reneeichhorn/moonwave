use std::collections::{HashMap, VecDeque};

use bitvec::prelude::*;
use itertools::Itertools;
use moonwave_common::Vector3;

#[derive(Clone)]
struct Cluster {
  /// starting chunk index.
  index_start: usize,
  /// total added chunks in this cluster.
  size: usize,
  /// free chunks in this cluster.
  free: Vec<usize>,
}

pub struct SharedAreaBuffer {
  options: SharedAreaBufferOptions,
  chunks: Vec<bool>,
  clusters: HashMap<Vector3<usize>, Vec<Cluster>>,
}

impl SharedAreaBuffer {
  pub fn new() -> Self {
    Self::with_options(SharedAreaBufferOptions::default())
  }

  pub fn with_options(options: SharedAreaBufferOptions) -> Self {
    let chunks = (0..options.total_chunks).map(|_| false).collect::<Vec<_>>();
    Self {
      options,
      chunks,
      clusters: HashMap::new(),
    }
  }

  pub fn find_best_chunk(&self) -> Option<(usize, usize)> {
    // Find longest following free chunks
    let mut cur_start = 0;
    let mut cur_length = 0;
    let mut active_best = None;

    for (index, reserved) in self.chunks.iter().enumerate() {
      if *reserved {
        if cur_length > 1 && cur_length > active_best.map(|(_, len)| len).unwrap_or(0) {
          active_best = Some((cur_start, cur_length));
          // early out optimization.
          if cur_length >= self.options.max_space_between {
            break;
          }
        }

        cur_start = index;
        cur_length = 1;
        continue;
      }

      cur_length += 1;
    }

    // Check for last item again.
    if cur_length > active_best.map(|(_, len)| len).unwrap_or(0) {
      active_best = Some((cur_start, cur_length));
    }

    // Slice in mid for best before and after chunks.
    active_best
      .map(|(start, length)| {
        // We at least need 2 or more chunks
        if length <= 1 {
          return None;
        }

        let half_length = length / 2;
        Some((start + half_length, length - half_length))
      })
      .flatten()
  }

  pub fn find_shortest_increasing_sequence(list: &[usize], min: usize) -> Option<usize> {
    if list.is_empty() {
      return None;
    }

    let mut sorted_list = list.iter().copied().collect_vec();
    sorted_list.sort_unstable();

    let mut found = vec![];
    let mut previous = sorted_list[0];
    let mut current_length = 1;

    for i in sorted_list.iter().skip(1) {
      if previous == *i + 1 {
        current_length += 1;
        previous += 1;
        continue;
      }

      // Early out for optimal end
      if current_length == min {
        return Some(i - current_length);
      }

      found.push((i - current_length, current_length));
      current_length = 1;
      previous = *i;
    }

    found.sort_unstable_by_key(|(_, length)| *length);
    found.get(0).map(|(x, _)| *x)
  }

  pub fn alloc(
    &mut self,
    amount: usize,
    position: Vector3<f32>,
  ) -> Option<SharedAreaBufferAllocation> {
    // Calculate position key
    let key_pos = Vector3::new(
      (position.x / self.options.cluster_size.x) as usize,
      (position.y / self.options.cluster_size.y) as usize,
      (position.z / self.options.cluster_size.z) as usize,
    );

    // Look for existing cluster
    let self_chunks = &mut self.chunks;
    let self_clusters = &mut self.clusters;

    if let Some(clusters) = self_clusters.get_mut(&key_pos) {
      for cluster in clusters {
        /*
        // Find cluster with free space
        let index = Self::find_shortest_increasing_sequence(&cluster.free, amount);
        if let Some(index) = index {
          // Retain free chunks that are no longer free.
          let range = index..index + amount;
          cluster.free.retain(|i| !range.contains(i));
          // Return reservation.
          return Some(SharedAreaBufferAllocation {
            chunk_start: cluster.index_start + index,
            chunk_length: amount,
            cluster_pos: key_pos,
            cluster_index_start: index,
          });
        }
        */

        // Check if cluster can be extended.
        /*
        let is_extentable = (0..amount).all(|i| {
          self_chunks[cluster.index_start + cluster.size + i]
            .owner
            .is_none()
        });
        if is_extentable {
          for i in 0..amount {
            self_chunks[cluster.index_start + cluster.size + i].owner = Some(key_pos);
          }
          cluster.size += amount;

          return Some(SharedAreaBufferAllocation {
            chunk_start: cluster.index_start + cluster.size - amount,
            chunk_length: amount,
            cluster_pos: key_pos,
            cluster_index_start: cluster.size - amount,
          });
        }
        */
      }
    }

    // Create cluster
    let (best_start, best_length) = self.find_best_chunk()?;
    if best_length < amount {
      // Not enough chunks are available.
      return None;
    }

    let reservable_amount = if self.options.cluster_chunk_size < amount {
      amount
    } else if self.options.cluster_chunk_size > best_length {
      best_length
    } else {
      self.options.cluster_chunk_size
    };
    let cluster = Cluster {
      index_start: best_start,
      size: reservable_amount,
      free: (amount..reservable_amount).collect::<Vec<_>>(),
    };

    // Store cluster
    if let Some(clusters) = self.clusters.get_mut(&key_pos) {
      clusters.push(cluster.clone());
    } else {
      self.clusters.insert(key_pos, vec![cluster.clone()]);
    }

    // Assign chunks to created cluster
    //self.chunks[cluster.index_start..(cluster.index_start + cluster.size)].set_all(true);
    for chunk in &mut self.chunks[cluster.index_start..(cluster.index_start + cluster.size)] {
      *chunk = true;
    }

    Some(SharedAreaBufferAllocation {
      chunk_start: cluster.index_start,
      chunk_length: amount,
      cluster_pos: key_pos,
      cluster_index_start: 0,
    })
  }

  pub fn free(&mut self, key: SharedAreaBufferAllocation) {
    let clusters = self.clusters.get_mut(&key.cluster_pos).unwrap();
    let (cluster_index, cluster) = clusters
      .iter_mut()
      .enumerate()
      .find(|(_, cluster)| {
        cluster.index_start >= key.chunk_start
          && (cluster.index_start + cluster.size) < key.chunk_start
      })
      .unwrap();

    // Case: cluster is completely empty and therefore we can remove it.
    if cluster.free.len() + key.chunk_length == cluster.size {
      // Clean chunk assignments.
      //self.chunks[cluster.index_start..(cluster.index_start + cluster.size)].set_all(false);
      for chunk in &mut self.chunks[cluster.index_start..(cluster.index_start + cluster.size)] {
        *chunk = false;
      }
      // Remove cluster
      clusters.remove(cluster_index);
      return;
    }

    // Case: Cluster is not empty but free can free some chunks
    for i in 0..key.chunk_length {
      cluster.free.push(i + key.cluster_index_start);
    }
  }
}

#[derive(Clone, Debug)]
pub struct SharedAreaBufferAllocation {
  pub chunk_start: usize,
  pub chunk_length: usize,
  cluster_pos: Vector3<usize>,
  cluster_index_start: usize,
}

pub struct SharedAreaBufferOptions {
  pub cluster_chunk_size: usize,
  pub total_chunks: usize,
  pub chunk_size: usize,
  pub max_space_between: usize,
  pub cluster_size: Vector3<f32>,
}

impl Default for SharedAreaBufferOptions {
  fn default() -> Self {
    Self {
      max_space_between: 2,
      cluster_chunk_size: 8,
      total_chunks: 128,
      chunk_size: 1024,
      cluster_size: Vector3::new(10.0, 10.0, 10.0),
    }
  }
}
