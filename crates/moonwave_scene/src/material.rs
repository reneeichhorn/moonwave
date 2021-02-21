use std::marker::PhantomData;

use moonwave_common::Vector4;
use moonwave_core::{BindGroupLayoutSingleton, Core, ShaderKind};
use moonwave_resources::{
  BindGroupLayout, PipelineLayout, PipelineLayoutDescriptor, ResourceRc, Shader,
};
use moonwave_shader::{
  Constant, Index, Multiply, ShaderGraph, ShaderType, Vector3Upgrade, VertexStruct,
};

use crate::{CameraUniform, ModelUniform};

pub struct Material<T: VertexStruct> {
  graph: ShaderGraph,
  vertex_in: Index,
  vertex_out: Index,
  color_outs: Vec<Index>,
  built: Option<BuiltMaterial>,
  _m: PhantomData<T>,
}

impl<T: VertexStruct + 'static> Material<T> {
  pub fn new() -> Self {
    let mut graph = ShaderGraph::new();
    let (vertex_in, vertex_out) = graph.add_vertex_attributes::<T>();
    let color_out = graph.add_color_output("Color", ShaderType::Float4);

    Self {
      vertex_in,
      vertex_out,
      graph,
      color_outs: vec![color_out],
      built: None,
      _m: PhantomData {},
    }
  }

  pub(crate) async fn build(&mut self, core: &Core) -> BuiltMaterial {
    if let Some(built) = &self.built {
      return built.clone();
    }

    // Build shaders from material.
    let graph = &mut self.graph;
    let (_camera_index, camera_in) = graph.add_uniform::<CameraUniform>();
    let (_model_index, model_in) = graph.add_uniform::<ModelUniform>();

    let color_constant = graph.add_node(Constant::new(Vector4::new(1.0, 0.0, 1.0, 1.0)));
    let matrix_multiply = graph.add_node(Multiply::new(ShaderType::Matrix4));
    let pos_multiply = graph.add_node(Multiply::new(ShaderType::Float4));
    let upgrade = graph.add_node(Vector3Upgrade {});

    // Hard coded color output.
    graph
      .connect(color_constant, Constant::OUTPUT, self.color_outs[0], 0)
      .unwrap();

    // Multiply matrices
    graph
      .connect(
        camera_in,
        CameraUniform::OUTPUT_PROJECTION_VIEW,
        matrix_multiply,
        Multiply::INPUT_A,
      )
      .unwrap();
    graph
      .connect(
        model_in,
        ModelUniform::OUTPUT_MATRIX,
        matrix_multiply,
        Multiply::INPUT_B,
      )
      .unwrap();

    // Make position
    graph
      .connect(self.vertex_in, 0, upgrade, Vector3Upgrade::INPUT)
      .unwrap();
    graph
      .connect(
        matrix_multiply,
        Multiply::OUTPUT,
        pos_multiply,
        Multiply::INPUT_A,
      )
      .unwrap();
    graph
      .connect(
        upgrade,
        Vector3Upgrade::OUTPUT,
        pos_multiply,
        Multiply::INPUT_B,
      )
      .unwrap();
    graph
      .connect(pos_multiply, Multiply::OUTPUT, self.vertex_out, 0)
      .unwrap();

    // Build shaders
    let built = graph.build(&self.color_outs);
    let vertex_shader = core
      .create_shader_from_glsl(built.vs.as_str(), "material_vs", ShaderKind::Vertex)
      .await
      .unwrap();
    let fragment_shader = core
      .create_shader_from_glsl(built.vs.as_str(), "material_fs", ShaderKind::Fragment)
      .await
      .unwrap();

    // Create layout
    let desc = PipelineLayoutDescriptor::new()
      .add_binding(CameraUniform::get_bind_group_lazy(core))
      .add_binding(ModelUniform::get_bind_group_lazy(core));
    let layout = core.create_pipeline_layout(desc).await;

    self.built = Some(BuiltMaterial {
      vertex_shader,
      fragment_shader,
      layout,
    });
    self.built.as_ref().unwrap().clone()
  }
}

#[derive(Clone)]
pub(crate) struct BuiltMaterial {
  pub(crate) vertex_shader: ResourceRc<Shader>,
  pub(crate) fragment_shader: ResourceRc<Shader>,
  pub(crate) layout: ResourceRc<PipelineLayout>,
}
