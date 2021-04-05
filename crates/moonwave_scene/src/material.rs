use std::collections::HashMap;
use std::sync::Arc;

use lazy_static::lazy_static;
use moonwave_core::{debug, Core, OnceCell, ShaderKind};
use moonwave_resources::{
  BindGroupLayout, BindGroupLayoutDescriptor, BindGroupLayoutEntryType, PipelineLayout,
  PipelineLayoutDescriptor, ResourceRc, Shader,
};
use moonwave_shader::{
  BuiltShaderBindGroup, BuiltShaderGraph, Index, InputPassthroughNode, Multiply, ShaderGraph,
  ShaderType, UniformStruct, Uuid, Vector3Upgrade,
};
use parking_lot::RwLock;

use crate::{CameraUniform, ModelUniform};

lazy_static! {
  pub static ref MATERIAL_UNIFORM_LAYOUT: ResourceRc<BindGroupLayout> = {
    let desc =
      BindGroupLayoutDescriptor::new().add_entry(0, BindGroupLayoutEntryType::UniformBuffer);
    Core::get_instance().create_bind_group_layout(desc)
  };
  pub static ref MATERIAL_TEXTURE_LAYOUT: ResourceRc<BindGroupLayout> = {
    let desc = BindGroupLayoutDescriptor::new()
      .add_entry(0, BindGroupLayoutEntryType::SingleTexture)
      .add_entry(1, BindGroupLayoutEntryType::Sampler);
    Core::get_instance().create_bind_group_layout(desc)
  };
}

pub struct Material {
  graph: RwLock<ShaderGraph>,
  built: RwLock<Option<Arc<BuiltMaterial>>>,
}

impl Material {
  pub fn new(graph: ShaderGraph) -> Self {
    Self {
      graph: RwLock::new(graph),
      built: RwLock::new(None),
    }
  }

  pub fn build(&self) -> Arc<BuiltMaterial> {
    let mut built_cache = self.built.write();
    if let Some(built) = &*built_cache {
      return built.clone();
    }

    // Build shaders
    let mut graph = self.graph.write();
    let outputs = graph
      .get_color_outputs()
      .iter()
      .map(|(_, _, index)| *index)
      .collect::<Vec<_>>();
    let built = graph.build(&outputs);

    debug!(
      "Build vertex shader {}\n\nBuild fragment shader: {}",
      built.vs, built.fs
    );

    // Compile
    let core = Core::get_instance();
    let vertex_shader = core
      .create_shader_from_glsl(built.vs.as_str(), "material_vs", ShaderKind::Vertex)
      .unwrap();
    let fragment_shader = core
      .create_shader_from_glsl(built.fs.as_str(), "material_fs", ShaderKind::Fragment)
      .unwrap();

    // Create layout
    let mut desc = PipelineLayoutDescriptor::new();
    for group in built.bind_groups.iter() {
      let layout = match group {
        BuiltShaderBindGroup::Uniform(_) => MATERIAL_UNIFORM_LAYOUT.clone(),
        BuiltShaderBindGroup::SampledTexture(_) => MATERIAL_TEXTURE_LAYOUT.clone(),
      };
      desc = desc.add_binding(layout);
    }
    let layout = core.create_pipeline_layout(desc);

    let built_material = Arc::new(BuiltMaterial {
      shader: built,
      vertex_shader,
      fragment_shader,
      layout,
    });
    *built_cache = Some(built_material.clone());
    built_material
  }
}

pub struct BuiltMaterial {
  pub shader: BuiltShaderGraph,
  pub vertex_shader: ResourceRc<Shader>,
  pub fragment_shader: ResourceRc<Shader>,
  pub layout: ResourceRc<PipelineLayout>,
}

pub struct PBRShaderNode {}

impl PBRShaderNode {
  pub const INPUT_POSITION: usize = 0;
  pub const INPUT_ALBEDO: usize = 1;

  pub fn new() -> PBRShaderNode {
    Self {}
  }

  pub fn build_graph() -> (ShaderGraph, Index) {
    // Basic shader graph that will be used as a sub graph only
    let mut graph = ShaderGraph::new();
    let vertex_out = graph.add_vertex_output_only();
    let color_out = graph.add_color_output("color", ShaderType::Float4);

    // Add passthrough node for pbr node inputs
    let input_index = graph.add_node(
      InputPassthroughNode::new()
        .add_input(ShaderType::Float3, "vec3(0, 0, 0)")
        .add_input(ShaderType::Float4, "vec4(0, 0, 0, 0)"),
    );

    // Build shaders from material.
    let (_, camera_in) = graph.add_uniform::<CameraUniform>("camera");
    let (_, model_in) = graph.add_uniform::<ModelUniform>("model");

    let matrix_multiply = graph.add_node(Multiply::new(ShaderType::Matrix4));
    let pos_multiply = graph.add_node(Multiply::new(ShaderType::Float4));
    let upgrade = graph.add_node(Vector3Upgrade {});

    // Color to color ouput
    graph
      .connect(input_index, Self::INPUT_ALBEDO, color_out, 0)
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
      .connect(
        input_index,
        Self::INPUT_POSITION,
        upgrade,
        Vector3Upgrade::INPUT,
      )
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
      .connect(pos_multiply, Multiply::OUTPUT, vertex_out, 0)
      .unwrap();

    (graph, input_index)
  }
}
