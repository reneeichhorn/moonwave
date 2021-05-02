use std::{collections::HashMap, hash::Hash};
use std::{hash::Hasher, sync::Arc};

use lazy_static::lazy_static;
use moonwave_core::{debug, Core, OnceCell, ShaderKind};
use moonwave_resources::{
  BindGroupLayout, BindGroupLayoutDescriptor, BindGroupLayoutEntryType, PipelineLayout,
  PipelineLayoutDescriptor, RenderPipeline, RenderPipelineDescriptor, ResourceRc, Shader,
  TextureFormat, VertexBuffer,
};
use moonwave_shader::{
  BuiltShaderBindGroup, BuiltShaderGraph, Construct, ConvertHomgenous, Deconstruct, Index,
  InputPassthroughNode, Multiply, ShaderGraph, ShaderNode, ShaderType, Vector3Upgrade,
};
use parking_lot::RwLock;

use crate::{CameraUniform, DirectionalLightShaderNode, LightsUniform, ModelUniform};

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

    // Build pbr pipeline.
    let pipeline = core.create_render_pipeline(
      RenderPipelineDescriptor::new(
        layout.clone(),
        built.vb.clone(),
        vertex_shader.clone(),
        fragment_shader.clone(),
      )
      .add_depth(TextureFormat::Depth32Float)
      .add_color_output(TextureFormat::Bgra8UnormSrgb),
    );

    let built_material = Arc::new(BuiltMaterial {
      shader: built,
      vertex_shader,
      fragment_shader,
      layout,
      pbr_pipeline: pipeline,
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
  pub pbr_pipeline: ResourceRc<RenderPipeline>,
}

impl Hash for BuiltMaterial {
  fn hash<H: Hasher>(&self, state: &mut H) {
    self.vertex_shader.hash(state);
  }
}
impl PartialEq for BuiltMaterial {
  fn eq(&self, other: &Self) -> bool {
    self.vertex_shader.eq(&other.vertex_shader)
  }
}
impl Eq for BuiltMaterial {}

pub struct PBRShaderNode {}

impl PBRShaderNode {
  pub const INPUT_POSITION: usize = 0;
  pub const INPUT_VNORMAL: usize = 1;
  pub const INPUT_VTANGENT: usize = 2;
  pub const INPUT_VBITANGENT: usize = 3;

  pub const INPUT_BASE_COLOR: usize = 4;
  pub const INPUT_METALLIC: usize = 5;
  pub const INPUT_ROUGHNESS: usize = 6;
  pub const INPUT_NORMAL: usize = 7;

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
        .add_input(ShaderType::Float3, "vec3(0, 0, 0)")
        .add_input(ShaderType::Float3, "vec3(0, 0, 0)")
        .add_input(ShaderType::Float3, "vec3(0, 0, 0)")
        .add_input(ShaderType::Float4, "vec4(0, 0, 0, 0)")
        .add_input(ShaderType::Float, "0.0")
        .add_input(ShaderType::Float, "0.0")
        .add_input(ShaderType::Float3, "vec3(0, 1.0, 0)"),
    );

    // Build shaders from material.
    let (_, camera_in) = graph.add_uniform::<CameraUniform>("camera");
    let (_, model_in) = graph.add_uniform::<ModelUniform>("model");
    let (_, lights_in) = graph.add_uniform::<LightsUniform>("lights");

    let matrix_multiply = graph.add_node(Multiply::new(ShaderType::Float4));
    let pos_multiply = graph.add_node(Multiply::new(ShaderType::Float4));
    let world_pos_homo = graph.add_node(ConvertHomgenous::new());
    let upgrade = graph.add_node(Vector3Upgrade {});
    let dir_light = graph.add_node(DirectionalLightShaderNode {});
    let mat_prepare = graph.add_node(MaterialPrepareNode {});
    let pixel = graph.add_node(PixelPrepareNode {});
    let normal = graph.add_node(NormalTransformNode {});
    let alpha_color = graph.add_node(Construct::new(ShaderType::Float4).unwrap());
    let base_color = graph.add_node(Deconstruct::new(ShaderType::Float4).unwrap());
    let alpha_discard = graph.add_node(AlphaDiscardNode(0.9));

    // Normal transform.
    graph
      .connect(
        input_index,
        Self::INPUT_VNORMAL,
        normal,
        NormalTransformNode::INPUT_NORMAL,
      )
      .unwrap();
    graph
      .connect(
        input_index,
        Self::INPUT_VTANGENT,
        normal,
        NormalTransformNode::INPUT_TANGENT,
      )
      .unwrap();
    graph
      .connect(
        input_index,
        Self::INPUT_VBITANGENT,
        normal,
        NormalTransformNode::INPUT_BITANGENT,
      )
      .unwrap();
    graph
      .connect(
        camera_in,
        CameraUniform::OUTPUT_VIEW,
        normal,
        NormalTransformNode::INPUT_CAMERA_VIEW,
      )
      .unwrap();
    graph
      .connect(
        model_in,
        ModelUniform::OUTPUT_MATRIX,
        normal,
        NormalTransformNode::INPUT_MODAL_VIEW,
      )
      .unwrap();
    graph
      .connect(
        world_pos_homo,
        ConvertHomgenous::OUTPUT,
        normal,
        NormalTransformNode::INPUT_POSITION,
      )
      .unwrap();

    // Material
    graph
      .connect(
        camera_in,
        CameraUniform::OUTPUT_POSITION,
        mat_prepare,
        MaterialPrepareNode::INPUT_CAMERA_POSITION,
      )
      .unwrap();
    graph
      .connect(
        world_pos_homo,
        ConvertHomgenous::OUTPUT,
        mat_prepare,
        MaterialPrepareNode::INPUT_WORLD_POSITION,
      )
      .unwrap();
    graph
      .connect(
        normal,
        NormalTransformNode::OUTPUT_NORMAL,
        mat_prepare,
        MaterialPrepareNode::INPUT_VERTEX_NORMAL,
      )
      .unwrap();
    graph
      .connect(
        normal,
        NormalTransformNode::OUTPUT_TANGENT,
        mat_prepare,
        MaterialPrepareNode::INPUT_VERTEX_TANGENT,
      )
      .unwrap();
    graph
      .connect(
        normal,
        NormalTransformNode::OUTPUT_BITANGENT,
        mat_prepare,
        MaterialPrepareNode::INPUT_VERTEX_BITANGENT,
      )
      .unwrap();
    graph
      .connect(
        input_index,
        Self::INPUT_NORMAL,
        mat_prepare,
        MaterialPrepareNode::INPUT_MATERIAL_NORMAL,
      )
      .unwrap();

    // Pixel
    graph
      .connect(
        input_index,
        Self::INPUT_BASE_COLOR,
        pixel,
        PixelPrepareNode::INPUT_BASE_COLOR,
      )
      .unwrap();
    graph
      .connect(
        input_index,
        Self::INPUT_METALLIC,
        pixel,
        PixelPrepareNode::INPUT_METALLIC,
      )
      .unwrap();
    graph
      .connect(
        input_index,
        Self::INPUT_ROUGHNESS,
        pixel,
        PixelPrepareNode::INPUT_ROUGHNESS,
      )
      .unwrap();

    // Light
    graph
      .connect(
        lights_in,
        LightsUniform::OUTPUT_DIRECTIONAL_LIGHTS,
        dir_light,
        DirectionalLightShaderNode::INPUT_LIGHTS,
      )
      .unwrap();
    graph
      .connect(
        pixel,
        PixelPrepareNode::OUTPUT_PIXEL,
        dir_light,
        DirectionalLightShaderNode::INPUT_PIXEL,
      )
      .unwrap();
    graph
      .connect(
        mat_prepare,
        MaterialPrepareNode::OUTPUT_SHADING_NORMAL,
        dir_light,
        DirectionalLightShaderNode::INPUT_SHADING_NORMAL,
      )
      .unwrap();
    graph
      .connect(
        mat_prepare,
        MaterialPrepareNode::OUTPUT_SHADING_VIEW,
        dir_light,
        DirectionalLightShaderNode::INPUT_SHADING_VIEW,
      )
      .unwrap();
    graph
      .connect(
        mat_prepare,
        MaterialPrepareNode::OUTPUT_SHADING_NOV,
        dir_light,
        DirectionalLightShaderNode::INPUT_SHADING_NOV,
      )
      .unwrap();

    // Color to color ouput
    graph
      .connect(
        input_index,
        Self::INPUT_BASE_COLOR,
        base_color,
        Deconstruct::INPUT,
      )
      .unwrap();
    graph
      .connect(
        base_color,
        Deconstruct::OUTPUT_W,
        alpha_discard,
        AlphaDiscardNode::INPUT_ALPHA,
      )
      .unwrap();
    graph
      .connect(
        dir_light,
        DirectionalLightShaderNode::OUTPUT_COLOR,
        alpha_color,
        Construct::INPUT_X,
      )
      .unwrap();
    graph
      .connect(
        alpha_discard,
        AlphaDiscardNode::OUTPUT_ALPHA,
        alpha_color,
        Construct::INPUT_Y,
      )
      .unwrap();
    graph
      .connect(alpha_color, Construct::OUTPUT, color_out, 0)
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
        model_in,
        ModelUniform::OUTPUT_MATRIX,
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
      .connect(
        pos_multiply,
        Multiply::OUTPUT,
        world_pos_homo,
        ConvertHomgenous::INPUT,
      )
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
        pos_multiply,
        Multiply::OUTPUT,
        matrix_multiply,
        Multiply::INPUT_B,
      )
      .unwrap();

    graph
      .connect(matrix_multiply, Multiply::OUTPUT, vertex_out, 0)
      .unwrap();

    (graph, input_index)
  }
}

#[derive(Debug)]
pub(crate) struct MaterialPrepareNode;
impl MaterialPrepareNode {
  const INPUT_WORLD_POSITION: usize = 0;
  const INPUT_CAMERA_POSITION: usize = 1;
  const INPUT_VERTEX_NORMAL: usize = 2;
  const INPUT_VERTEX_TANGENT: usize = 3;
  const INPUT_VERTEX_BITANGENT: usize = 4;
  const INPUT_MATERIAL_NORMAL: usize = 5;
  const OUTPUT_SHADING_VIEW: usize = 0;
  const OUTPUT_SHADING_NORMAL: usize = 1;
  const OUTPUT_SHADING_NOV: usize = 2;
}

impl ShaderNode for MaterialPrepareNode {
  fn generate_global_code(
    &self,
    _inputs: &[Option<String>],
    _outputs: &[Option<String>],
    output: &mut String,
  ) {
    let global = include_str!("./helper.frag");
    *output += global;
  }

  fn get_outputs(&self) -> Vec<ShaderType> {
    vec![ShaderType::Float3, ShaderType::Float3, ShaderType::Float]
  }

  fn generate(&self, inputs: &[Option<String>], outputs: &[Option<String>], output: &mut String) {
    *output += format!(
      r#"
        // Camera - Position view direction.
        vec3 {} = normalize({} - {});

        // Normal
        mat3 tbn = mat3({}, {}, {});
        vec3 {} = normalize(tbn * {});
        /*
        if ({}.z <= 0) {{
          {} *= -1.0;
        }}
        */

        // NOV
        float {} = clampNoV(dot({}, {}));
      "#,
      // Camera
      outputs[Self::OUTPUT_SHADING_VIEW].as_ref().unwrap(),
      inputs[Self::INPUT_CAMERA_POSITION].as_ref().unwrap(),
      inputs[Self::INPUT_WORLD_POSITION].as_ref().unwrap(),
      // Normal
      inputs[Self::INPUT_VERTEX_TANGENT].as_ref().unwrap(),
      inputs[Self::INPUT_VERTEX_BITANGENT].as_ref().unwrap(),
      inputs[Self::INPUT_VERTEX_NORMAL].as_ref().unwrap(),
      outputs[Self::OUTPUT_SHADING_NORMAL].as_ref().unwrap(),
      inputs[Self::INPUT_MATERIAL_NORMAL].as_ref().unwrap(),
      outputs[Self::OUTPUT_SHADING_NORMAL].as_ref().unwrap(),
      outputs[Self::OUTPUT_SHADING_NORMAL].as_ref().unwrap(),
      // NOV
      outputs[Self::OUTPUT_SHADING_NOV].as_ref().unwrap(),
      outputs[Self::OUTPUT_SHADING_NORMAL].as_ref().unwrap(),
      outputs[Self::OUTPUT_SHADING_VIEW].as_ref().unwrap(),
    )
    .as_str();
  }
}

#[derive(Debug)]
pub(crate) struct NormalTransformNode {}
impl NormalTransformNode {
  const INPUT_CAMERA_VIEW: usize = 0;
  const INPUT_MODAL_VIEW: usize = 1;
  const INPUT_NORMAL: usize = 2;
  const INPUT_TANGENT: usize = 3;
  const INPUT_BITANGENT: usize = 4;
  const INPUT_POSITION: usize = 5;
  const OUTPUT_NORMAL: usize = 0;
  const OUTPUT_TANGENT: usize = 1;
  const OUTPUT_BITANGENT: usize = 2;
}
impl ShaderNode for NormalTransformNode {
  fn generate(&self, inputs: &[Option<String>], outputs: &[Option<String>], output: &mut String) {
    *output += format!(
      r#"
        float flipper = gl_FrontFacing ? 1.0 : -1.0;
        mat3 N = mat3(transpose(inverse({} * {})));
        vec3 {} = normalize(N * (flipper * {}));
        vec3 {} = normalize(N * (flipper * {}));
        vec3 {} = normalize(N * (flipper * {}));
      "#,
      inputs[Self::INPUT_MODAL_VIEW].as_ref().unwrap(),
      inputs[Self::INPUT_CAMERA_VIEW].as_ref().unwrap(),
      outputs[Self::OUTPUT_NORMAL].as_ref().unwrap(),
      inputs[Self::INPUT_NORMAL].as_ref().unwrap(),
      outputs[Self::OUTPUT_TANGENT].as_ref().unwrap(),
      inputs[Self::INPUT_TANGENT].as_ref().unwrap(),
      outputs[Self::OUTPUT_BITANGENT].as_ref().unwrap(),
      inputs[Self::INPUT_BITANGENT].as_ref().unwrap(),
    )
    .as_str();
  }
}

#[derive(Debug)]
pub(crate) struct PixelPrepareNode {}
impl PixelPrepareNode {
  const INPUT_BASE_COLOR: usize = 0;
  const INPUT_METALLIC: usize = 1;
  const INPUT_ROUGHNESS: usize = 2;
  const OUTPUT_PIXEL: usize = 0;
}
impl ShaderNode for PixelPrepareNode {
  fn generate_global_code(
    &self,
    _inputs: &[Option<String>],
    _outputs: &[Option<String>],
    output: &mut String,
  ) {
    *output += r#"
      struct Pixel {
        vec3 f0;
        vec3 diffuse;
        float roughness;
        vec3 energyCompensation;
      };
    "#;
  }

  fn get_outputs(&self) -> Vec<ShaderType> {
    vec![ShaderType::Struct("Pixel")]
  }

  fn generate(&self, inputs: &[Option<String>], outputs: &[Option<String>], output: &mut String) {
    *output += format!(
      r#"
      Pixel pixel;

      // Diffuse
      pixel.diffuse = computeDiffuseColor({}, {});

      // Frenal
      float reflectance = computeDielectricF0(0.0);
      pixel.f0 = computeF0({}, {}, reflectance);

      // Roughness
      float perceptualRoughness = {};
      perceptualRoughness = clamp(perceptualRoughness, MIN_PERCEPTUAL_ROUGHNESS, 1.0);
      pixel.roughness = perceptualRoughnessToRoughness(perceptualRoughness);

      // Energy compensation
      pixel.energyCompensation = vec3(1.0);

      Pixel {} = pixel;
    "#,
      inputs[Self::INPUT_BASE_COLOR].as_ref().unwrap(),
      inputs[Self::INPUT_METALLIC].as_ref().unwrap(),
      inputs[Self::INPUT_BASE_COLOR].as_ref().unwrap(),
      inputs[Self::INPUT_METALLIC].as_ref().unwrap(),
      inputs[Self::INPUT_ROUGHNESS].as_ref().unwrap(),
      outputs[Self::OUTPUT_PIXEL].as_ref().unwrap(),
    )
    .as_str()
  }
}

#[derive(Debug)]
pub(crate) struct AlphaDiscardNode(f32);

impl AlphaDiscardNode {
  const INPUT_ALPHA: usize = 0;
  const OUTPUT_ALPHA: usize = 0;
}

impl ShaderNode for AlphaDiscardNode {
  fn generate(&self, inputs: &[Option<String>], outputs: &[Option<String>], output: &mut String) {
    *output += format!(
      r#"
        if ({} <= {}) {{
          discard;
        }}
        float {} = {};
      "#,
      inputs[Self::INPUT_ALPHA].as_ref().unwrap(),
      self.0,
      outputs[Self::OUTPUT_ALPHA].as_ref().unwrap(),
      inputs[Self::INPUT_ALPHA].as_ref().unwrap(),
    )
    .as_str();
  }
}
