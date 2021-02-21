use crate::{CommandEncoder, CommandEncoderOutput};
use futures::{executor::block_on, future::join_all, Future};
use generational_arena::Arena;
use moonwave_resources::{Buffer, ResourceRc, TextureView};
use multimap::MultiMap;
use parking_lot::{RwLock, RwLockReadGuard};
use std::pin::Pin;
use std::sync::Arc;

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

#[derive(Clone)]
pub enum FrameNodeValue {
  Buffer(ResourceRc<Buffer>),
  Texture(ResourceRc<TextureView>),
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
        Self::traverse_node(levels_map, nodes, edges, inner_node, next_level);
      }
    }

    // Store traversed node at level.
    levels_map.insert(level, traversed_node);
  }

  /// Executes the graph using the given scheduler.
  pub fn execute<T: DeviceHost, F>(
    &mut self,
    sc_frame: Arc<wgpu::SwapChainFrame>,
    device_host: Arc<T>,
    scheduler: F,
  ) where
    F: Fn(
      Pin<Box<dyn Future<Output = CommandEncoderOutput> + Send + Sync>>,
    ) -> Pin<Box<dyn Future<Output = CommandEncoderOutput>>>,
  {
    {
      // Gain read access to nodes and connections.
      let nodes = self.node_arena.read();
      let edges = self.edges_arena.read();

      // Start traversing from end.
      Self::traverse_node(&mut self.levels_map, &nodes, &edges, self.end_node, 0);

      // Execute in levels order
      let mut all_levels = self.levels_map.keys().cloned().collect::<Vec<_>>();
      all_levels.sort_unstable();

      for level in all_levels.into_iter().rev() {
        optick::event!("FrameGraph::execute_level");
        optick::tag!("level", level as u32);

        // Get rid of duplicated nodes.
        let nodes_in_level = self.levels_map.get_vec_mut(&level).unwrap();
        nodes_in_level.sort_unstable_by_key(|x| x.index);
        nodes_in_level.dedup_by_key(|x| x.index);

        let mut output_map_offset = 0;

        // Execute
        let mut futures = Vec::with_capacity(nodes_in_level.len());
        for traversed_node in nodes_in_level {
          optick::event!("FrameGraph::node");

          // Prepare node execution
          let node = nodes.get(traversed_node.index).unwrap();
          optick::tag!("name", node.name);
          let node_trait = node.node.clone();
          let label = format!("NodeCommandEncoder_{}", node.name);

          // Map outputs -> inputs.
          let output_map = &self.output_map;
          let inputs = traversed_node
            .inputs
            .iter()
            .map(|input| input.map(|(level, output_index, _)| &output_map[level][output_index]))
            .map(|input| match input {
              Some(Some(rf)) => Some(rf.clone()),
              _ => None,
            })
            .collect::<Vec<_>>();
          let mut outputs =
            self.output_map[level][output_map_offset..MAX_INPUT_OUTPUTS_PER_NODE].to_vec();

          let device_host_cloned = device_host.clone();
          let sc_cloned = sc_frame.clone();
          let fut = scheduler(Box::pin(async move {
            optick::event!("FrameGraph::record_commands");
            optick::tag!("name", label);

            // Execute node asynchronisly.
            node_trait.execute_raw(
              &inputs,
              &mut outputs,
              device_host_cloned.get_device(),
              device_host_cloned.get_queue(),
              &*sc_cloned,
            )
          }));

          output_map_offset += MAX_INPUT_OUTPUTS_PER_NODE;
          futures.push(fut);
        }

        let encoder_outputs = {
          optick::event!("FrameGraph::barrier_level");
          optick::tag!("level", level as u32);
          block_on(join_all(futures))
        };
        {
          optick::event!("FrameGraph::submit_level");
          optick::tag!("level", level as u32);
          let mut buffers = Vec::with_capacity(encoder_outputs.len());
          for out in encoder_outputs {
            buffers.push(out.command_buffer)
          }
          device_host.get_queue().submit(buffers);
        }
      }
    }

    // Reset
    self.reset();
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

struct TraversedGraphNode {
  index: Index,
  inputs: [Option<(usize, usize, Index)>; MAX_INPUT_OUTPUTS_PER_NODE],
}

pub trait DeviceHost: Send + Sync + 'static {
  fn get_device(&self) -> &wgpu::Device;
  fn get_queue(&self) -> &wgpu::Queue;
}
