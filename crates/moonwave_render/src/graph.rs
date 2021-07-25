use crate::{CommandEncoder, CommandEncoderOutput};
use generational_arena::Arena;
use moonwave_resources::{BindGroup, Buffer, ResourceRc, SampledTexture, TextureView};
use multimap::MultiMap;
use parking_lot::{RwLock, RwLockReadGuard};
use rayon::{prelude::*, ThreadPool};
use std::{
  collections::HashMap,
  fmt::{Debug, Formatter},
  sync::Arc,
};

pub use generational_arena::Index;

pub trait FrameGraphNode: Send + Sync + 'static {
  fn execute(
    &self,
    _inputs: &[Option<FrameNodeValue>],
    _outputs: &mut [Option<FrameNodeValue>],
    _encoder: &mut CommandEncoder,
  ) {
  }

  fn execute_raw(
    &self,
    inputs: &[Option<FrameNodeValue>],
    outputs: &mut [Option<FrameNodeValue>],
    device: &wgpu::Device,
    _queue: &wgpu::Queue,
    _sc_frame: &wgpu::SwapChainFrame,
  ) -> CommandEncoderOutput {
    let mut encoder = CommandEncoder::new(device, "NodeGraphEncoder");
    self.execute(inputs, outputs, &mut encoder);
    encoder.finish()
  }
}

const MAX_LAYERS: usize = 8;
const MAX_NODES_PER_LAYER: usize = 8;
const MAX_INPUT_OUTPUTS_PER_NODE: usize = 16;

struct ConnectedNode {
  name: String,
  node: Arc<dyn FrameGraphNode>,
  inputs: [Option<Index>; MAX_INPUT_OUTPUTS_PER_NODE],
}

struct ConnectedEdges {
  owner_node_index: Index,
  output_index: usize,
}

pub struct FrameGraph {
  node_arena: RwLock<Arena<ConnectedNode>>,
  edges_arena: RwLock<Arena<ConnectedEdges>>,
  end_node: Index,
  output_map: Vec<Vec<Option<FrameNodeValue>>>,
  levels_map: MultiMap<usize, TraversedGraphNode>,
  traversed_node_cache: HashMap<Index, usize>,
}

impl FrameGraph {
  /// Creates a new empty graph.
  pub fn new<T: FrameGraphNode>(end_node: T) -> Self {
    let mut node_arena = Arena::with_capacity(MAX_LAYERS * MAX_NODES_PER_LAYER);
    let end_node = node_arena.insert(ConnectedNode {
      name: "EndNode".to_string(),
      node: Arc::new(end_node),
      inputs: [None; MAX_INPUT_OUTPUTS_PER_NODE],
    });

    Self {
      node_arena: RwLock::new(node_arena),
      edges_arena: RwLock::new(Arena::with_capacity(
        MAX_LAYERS * MAX_INPUT_OUTPUTS_PER_NODE * MAX_NODES_PER_LAYER,
      )),
      output_map: vec![vec![None; MAX_NODES_PER_LAYER * MAX_INPUT_OUTPUTS_PER_NODE]; MAX_LAYERS],
      levels_map: MultiMap::with_capacity(MAX_LAYERS),
      traversed_node_cache: HashMap::with_capacity(
        MAX_LAYERS * MAX_INPUT_OUTPUTS_PER_NODE * MAX_NODES_PER_LAYER,
      ),
      end_node,
    }
  }

  /// Returns the end node.
  pub fn get_end_node(&self) -> Index {
    self.end_node
  }

  /// Resets the frame graph by removing all nodes and sets up a new end node.
  pub fn reset(&mut self) {
    let mut nodes = self.node_arena.write();
    let end_node_impl = nodes.get(self.end_node).unwrap().node.clone();

    nodes.clear();
    self.traversed_node_cache.clear();
    self.edges_arena.write().clear();
    self.end_node = nodes.insert(ConnectedNode {
      name: "EndNode".to_string(),
      node: end_node_impl,
      inputs: [None; MAX_INPUT_OUTPUTS_PER_NODE],
    });
  }

  /// Add a new node into the graph.
  pub fn add_node<T: FrameGraphNode>(&self, node: T, name: &str) -> Index {
    self.node_arena.write().insert(ConnectedNode {
      name: name.to_string(),
      node: Arc::new(node),
      inputs: [None; MAX_INPUT_OUTPUTS_PER_NODE],
    })
  }

  /// Connects one nodes output to another nodes input.
  pub fn connect(
    &self,
    source: Index,
    source_output: usize,
    destination: Index,
    destination_input: usize,
  ) -> Result<(), GraphConnectError> {
    // Validate connection parameters.
    if destination_input >= MAX_INPUT_OUTPUTS_PER_NODE {
      return Err(GraphConnectError::MaximumInputsReached);
    };
    if source_output >= MAX_INPUT_OUTPUTS_PER_NODE {
      return Err(GraphConnectError::MaximumInputsReached);
    };

    let mut edges = self.edges_arena.write();
    let mut nodes = self.node_arena.write();
    let destination_node = nodes
      .get_mut(destination)
      .ok_or(GraphConnectError::InvalidDestination)?;

    // Target input is already connected.
    if destination_node.inputs[destination_input].is_some() {
      return Err(GraphConnectError::AlreadyConnected);
    }

    // Target input is empty so simply create the connection.
    let edge = edges.insert(ConnectedEdges {
      owner_node_index: source,
      output_index: source_output,
    });
    destination_node.inputs[destination_input] = Some(edge);

    Ok(())
  }

  fn traverse_node(
    cache: &mut HashMap<Index, usize>,
    levels_map: &mut MultiMap<usize, TraversedGraphNode>,
    nodes: &RwLockReadGuard<Arena<ConnectedNode>>,
    edges: &RwLockReadGuard<Arena<ConnectedEdges>>,
    node_index: Index,
    level: usize,
  ) {
    //Build traverse node with input/output mapping info.
    let mut traversed_node = TraversedGraphNode {
      index: node_index,
      inputs: [None; MAX_INPUT_OUTPUTS_PER_NODE],
    };

    // Remove from dependencies from all levels lower
    let mut has_retained = false;
    for l in level..0 {
      // Remove previous traversed node from level.
      let vec = levels_map.get_vec_mut(&l).unwrap();
      let before_len = vec.len();
      vec.retain(|x| x.index != node_index);
      if before_len != vec.len() {
        has_retained = true;
      }
    }

    // Update all inputs that still reference kicked out node.
    if has_retained {
      for l in level..0 {
        let vec = levels_map.get_vec_mut(&l).unwrap();
        for node in vec {
          for input in &mut node.inputs {
            if let Some((nlevel, _, index)) = input {
              if index == &node_index {
                *nlevel = level;
              }
            }
          }
        }
      }
    }

    // Loop through all inputs
    let next_level = level + 1;
    let node = nodes.get(node_index).unwrap();
    for (input_index, input) in node.inputs.iter().enumerate() {
      if let Some(input) = input {
        let edge = edges.get(*input).unwrap();
        let inner_node = edge.owner_node_index;
        traversed_node.inputs[input_index] = Some((next_level, edge.output_index, inner_node));
        Self::traverse_node(cache, levels_map, nodes, edges, inner_node, next_level);
      }
    }

    // Store traversed node at level.
    //let traversed_index = levels_map.get_vec(&level).map(|x| x.len()).unwrap_or(0);
    //cache.insert(node_index, traversed_index);
    // TODO: Due to retaining this index breaks currently :'(
    levels_map.insert(level, traversed_node);
  }

  /// Executes the graph using the given scheduler.
  pub fn execute<T: DeviceHost>(
    &mut self,
    sc_frame: Arc<wgpu::SwapChainFrame>,
    device_host: &'static T,
    pool: &ThreadPool,
  ) {
    {
      {
        optick::event!("FrameGraph::traverse");
        // Gain read access to nodes and connections.
        let nodes = self.node_arena.read();
        let edges = self.edges_arena.read();

        // Start traversing from end.
        self.levels_map.clear();
        Self::traverse_node(
          &mut self.traversed_node_cache,
          &mut self.levels_map,
          &nodes,
          &edges,
          self.end_node,
          0,
        );
      }
      let cache = &mut self.traversed_node_cache;

      // Create async executer.
      let mut local_pool = futures::executor::LocalPool::new();
      let local_spawner = local_pool.spawner();

      // Execute in levels order
      let mut all_levels = self.levels_map.keys().cloned().collect::<Vec<_>>();
      all_levels.sort_unstable();

      let max_levels = all_levels.len();
      for level in all_levels.into_iter().rev() {
        optick::event!("FrameGraph::execute_level");
        optick::tag!("level", level as u32);

        // Get rid of duplicated nodes.
        let mut nodes_in_level = self.levels_map.get_vec_mut(&level).unwrap().clone();
        nodes_in_level.sort_unstable_by_key(|x| x.index);
        nodes_in_level.dedup_by_key(|x| x.index);

        // Build cache for this level
        for (index, node) in nodes_in_level.iter().enumerate() {
          cache.insert(node.index, index);
        }

        // Get chunks
        let nodes = self.node_arena.read();
        let read_nodes = nodes_in_level
          .iter()
          .map(|node| (nodes.get(node.index).unwrap(), node.inputs))
          .collect::<Vec<_>>();

        let mut empty = [Vec::with_capacity(0)];
        #[allow(clippy::type_complexity)]
        let (outputs, previous_outputs): (
          &mut [Vec<Option<FrameNodeValue>>],
          &mut [Vec<Option<FrameNodeValue>>],
        ) = if level == (max_levels - 1) {
          (&mut self.output_map, &mut empty)
        } else {
          self.output_map.split_at_mut(level + 1)
        };

        let outputs_per_node = outputs[outputs.len() - 1]
          .chunks_mut(MAX_INPUT_OUTPUTS_PER_NODE)
          .enumerate()
          .collect::<Vec<_>>();

        // Execute
        let encoder_outputs = pool.install(|| {
          read_nodes
            .par_iter()
            .zip(outputs_per_node)
            .enumerate()
            .map(|(_i, ((node, inputs), (_oi, outputs)))| {
              optick::event!("FrameGraph::node");

              // Prepare node execution
              optick::tag!("name", node.name);
              let node_trait = node.node.clone();
              let label = format!("NodeCommandEncoder_{}", node.name);

              // Map outputs -> inputs.
              /*
              for (idx, input) in inputs.iter().enumerate() {
                if let Some((target_level, output_index, node_index)) = input {
                  let i = cache.get(&node_index).unwrap();
                  println!(
                    "Mapping input #{} to level = {} ({}) and index = {} ({}, {})",
                    idx,
                    target_level,
                    previous_outputs.len() - (target_level - level),
                    i * MAX_INPUT_OUTPUTS_PER_NODE + output_index,
                    i,
                    output_index
                  );
                } else {
                  println!("Mapping input #{} to None", i);
                }
              }
              */
              let inputs = inputs
                .iter()
                .map(|input| {
                  input.map(|(target_level, output_index, node_index)| {
                    let i = cache.get(&node_index).unwrap();
                    &previous_outputs[previous_outputs.len() - (target_level - level)]
                      [i * MAX_INPUT_OUTPUTS_PER_NODE + output_index]
                  })
                })
                .map(|input| match input {
                  Some(Some(rf)) => Some(rf.clone()),
                  _ => None,
                })
                .collect::<Vec<_>>();

              let sc_cloned = sc_frame.clone();
              let out = {
                optick::event!("FrameGraph::record_commands");
                optick::tag!("name", label);

                // Execute node asynchronisly.
                node_trait.execute_raw(
                  &inputs,
                  outputs,
                  device_host.get_device(),
                  device_host.get_queue(),
                  &*sc_cloned,
                )
              };

              out
            })
            .collect::<Vec<_>>()
        });

        {
          optick::event!("FrameGraph::submit_level");
          optick::tag!("level", level as u32);
          let mut buffers = Vec::with_capacity(encoder_outputs.len());

          for out in encoder_outputs {
            if let Some(buffer) = out.command_buffer {
              buffers.push(buffer);
            }
          }
          device_host.get_queue().submit(buffers);
        }
      }
    }

    // Reset
    optick::event!("FrameGraph::reset");
    self.reset();
  }
}

#[derive(Clone)]
pub enum FrameNodeValue {
  Buffer(ResourceRc<Buffer>),
  BindGroup(ResourceRc<BindGroup>),
  TextureView(ResourceRc<TextureView>),
  SampledTexture(SampledTexture),
}

impl std::fmt::Debug for FrameNodeValue {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::Buffer(_) => f.write_str("Buffer"),
      Self::BindGroup(_) => f.write_str("BindGroup"),
      Self::TextureView(_) => f.write_str("Texture"),
      Self::SampledTexture(_) => f.write_str("SampledTexture"),
    }
  }
}

use thiserror::Error;

#[derive(Error, Debug)]
pub enum GraphConnectError {
  #[error("The target node has reached its input limit")]
  MaximumInputsReached,
  #[error("The source node has reached its outputs limit")]
  MaximumOutputsReached,
  #[error("The target node does not exist")]
  InvalidDestination,
  #[error("The target nodes input is already connected")]
  AlreadyConnected,
}

#[derive(Clone)]
struct TraversedGraphNode {
  index: Index,
  inputs: [Option<(usize, usize, Index)>; MAX_INPUT_OUTPUTS_PER_NODE],
}

pub trait DeviceHost: Send + Sync + 'static {
  fn get_device(&self) -> &wgpu::Device;
  fn get_queue(&self) -> &wgpu::Queue;
}

macro_rules! impl_get_node_specific {
  ($getter:ident, $ty:ident, $rty:ty) => {
    impl FrameNodeValue {
      pub fn $getter(&self) -> &$rty {
        match self {
          FrameNodeValue::$ty(group) => group,
          _ => panic!(
            "Unexpected frame node value, expected '{}' but received '{:?}'",
            stringify!($ty),
            self
          ),
        }
      }
    }
  };
}

impl_get_node_specific!(get_bind_group, BindGroup, ResourceRc<BindGroup>);
impl_get_node_specific!(get_texture_view, TextureView, ResourceRc<TextureView>);
impl_get_node_specific!(get_sampled_texture, SampledTexture, SampledTexture);
