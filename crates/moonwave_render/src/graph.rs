use crate::{BufferRef, CommandEncoder, RenderPassCommandEncoder, TextureViewRef};
use generational_arena::{Arena, Index};

pub trait FrameGraphNode<B: BufferRef, T: TextureViewRef> {
  fn execute(
    self,
    inputs: &[FrameNodeValue<B, T>],
    output: &mut Vec<FrameNodeValue<B, T>>,
    encoder: &CommandEncoder,
  );
}

pub enum FrameNodeValue<B: BufferRef, T: TextureViewRef> {
  Buffer(B),
  Texture(T),
}

struct ConnectedNode<B: BufferRef, T: TextureViewRef> {
  node: Box<dyn FrameGraphNode<B, T>>,
  inputs: Vec<Index>,
}

struct ConnectedEdges {
  owner_node_index: Index,
  output_index: usize,
  target_node_index: Index,
  input_index: usize,
}

pub struct FrameGraph<B: BufferRef, T: TextureViewRef> {
  node_arena: Arena<ConnectedNode<B, T>>,
  edges_arena: Arena<ConnectedEdges>,
}

struct PresentToScreenNode;

impl PresentToScreenNode {
  const INPUT_TEXTURE_VIEW: usize = 0;
}

