use generational_arena::Arena;
use lazy_static::lazy_static;
use lyon::{lyon_tessellation::VertexBuffers, math::Point};
use moonwave_common::*;
use moonwave_core::*;
use moonwave_render::*;
use moonwave_resources::*;
use moonwave_scene::{
  BuiltMaterial, GenericUniform, Material, StagedBuffer, StagedBufferAccessor, Uniform,
};
use moonwave_shader::*;
use parking_lot::Mutex;
use send_wrapper::SendWrapper;
use std::sync::{
  atomic::{AtomicBool, Ordering},
  Arc,
};

use crate::{Component, UIRenderer};

#[uniform]
struct TransformUniform {
  view: Matrix4<f32>,
}

#[vertex]
struct ColoredShapeVertex {
  position: Vector3<f32>,
  color: Vector4<f32>,
}

struct RenderResources {
  vertex_buffer: StagedBuffer<ColoredShapeVertex>,
  index_buffer: StagedBuffer<u16>,
  transform: Uniform<TransformUniform>,
  vs_transform: ResourceRc<BindGroup>,
  shader_colored_shape: Arc<BuiltMaterial>,
  pipeline_colored_shape: ResourceRc<RenderPipeline>,
  ui_texture: Arc<TextureGeneratorHost>,
  active_indices: u16,
}

impl RenderResources {
  pub fn new() -> Self {
    // Build resources
    let transform = Uniform::new(TransformUniform {
      view: Matrix4::identity(),
    });
    let vs_transform = transform.get_bind_group();

    // Build colored shape shader.
    let shader_colored_shape = {
      // Graph setup
      let mut graph = ShaderGraph::new();
      let color_out = graph.add_color_output("color", ShaderType::Float4);
      let (_, transform_out) = graph.add_uniform::<TransformUniform>("transform");
      let (vertex_in, vertex_out) = graph.add_vertex_attributes::<ColoredShapeVertex>();

      // Nodes
      let mul = graph.add_node(Multiply::new(ShaderType::Float4));
      let upgrade = graph.add_node(Vector3Upgrade {});

      // Connect
      graph
        .connect(vertex_in, ColoredShapeVertex::OUTPUT_COLOR, color_out, 0)
        .unwrap();
      graph
        .connect(
          vertex_in,
          ColoredShapeVertex::OUTPUT_POSITION,
          upgrade,
          Vector3Upgrade::INPUT,
        )
        .unwrap();
      graph
        .connect(upgrade, Vector3Upgrade::OUTPUT, mul, Multiply::INPUT_B)
        .unwrap();
      graph
        .connect(
          transform_out,
          TransformUniform::OUTPUT_VIEW,
          mul,
          Multiply::INPUT_A,
        )
        .unwrap();
      graph.connect(mul, Multiply::OUTPUT, vertex_out, 0).unwrap();

      // Build shader
      Material::new(graph).build()
    };

    // Build pipeline
    let pipeline_colored_shape = Core::get_instance().create_render_pipeline(
      RenderPipelineDescriptor::new(
        shader_colored_shape.layout.clone(),
        ColoredShapeVertex::generate_buffer(),
        shader_colored_shape.vertex_shader.clone(),
        shader_colored_shape.fragment_shader.clone(),
      )
      .add_color_output(TextureFormat::Bgra8UnormSrgb),
    );

    // Build and reserve buffers
    let vertex_buffer = StagedBuffer::new(2048, BufferUsage::VERTEX);
    let index_buffer = StagedBuffer::new(1024, BufferUsage::INDEX);

    // Build UI texture
    let ui_texture =
      TextureGeneratorHost::new(TextureSize::FullScreen, TextureFormat::Bgra8UnormSrgb);

    Self {
      transform,
      vs_transform,
      shader_colored_shape,
      pipeline_colored_shape,
      vertex_buffer,
      index_buffer,
      ui_texture,
      active_indices: 0,
    }
  }
}

pub struct UIExtension {
  resources: Mutex<Option<RenderResources>>,
  _renderer: SendWrapper<UIRenderer>,
}

impl UIExtension {
  pub fn new(c: impl Component + 'static) -> Self {
    let renderer = UIRenderer::new(c);
    renderer.mount();

    Self {
      resources: Mutex::new(None),
      _renderer: SendWrapper::new(renderer),
    }
  }
}

impl Extension for UIExtension {
  fn before_tick(&self) {
    optick::event!("moonwave_ui::UIExtension::before_frame");

    // Build or update resources
    let mut resources_lock = self.resources.lock();
    let resources = resources_lock.get_or_insert_with(|| {
      optick::event!("moonwave_ui::UIExtension::create_resources");
      RenderResources::new()
    });

    // Update geometry
    {
      optick::event!("moonwave_ui::UIExtension::update_geometry");
      let dirty = SHAPE_MANAGER.dirty.load(Ordering::Relaxed);
      if dirty {
        // Write to staging buffer
        let mut vertex_buffer = resources.vertex_buffer.get_mut();
        vertex_buffer.clear();
        let mut index_buffer = resources.index_buffer.get_mut();
        index_buffer.clear();

        // Build colored geometry
        let mut offset = 0;
        let shapes = SHAPE_MANAGER.colored_shapes.lock();
        for (_, shape) in shapes.iter() {
          // Vertices
          let vertices = shape.geometry.vertices.iter().map(|v| ColoredShapeVertex {
            position: Vector3::new(v.x, v.y, 0.0),
            color: shape.color,
          });
          vertex_buffer.extend(vertices);

          // Indices
          let indices = shape.geometry.indices.iter().map(move |i| *i + offset);
          offset += shape.geometry.vertices.len() as u16;
          index_buffer.extend(indices);
        }

        resources.active_indices = index_buffer.len() as u16;
      }
    }

    // Update transform uniform
    {
      let size = Core::get_instance().get_swap_chain_size();
      let mut transform = resources.transform.get_mut();
      transform.view = ortho(0.0, size.x as f32, size.y as f32, 0.0, -100.0, 100.0);
    }

    // Build frame graph
    if resources.active_indices > 0 {
      optick::event!("moonwave_ui::UIExtension::build_frame");

      let graph = Core::get_instance().get_frame_graph();
      let texture_in = graph.add_node(resources.ui_texture.create_node(), "UITextureHost");
      let texture_out = graph.add_node(
        ColoredShapeRenderNode {
          indices: resources.active_indices,
          vb: resources.vertex_buffer.get_accessor(),
          ib: resources.index_buffer.get_accessor(),
          transform: resources.transform.as_generic(),
          pipeline: resources.pipeline_colored_shape.clone(),
        },
        "UIColoredShape",
      );

      graph
        .connect(
          texture_in,
          TextureGeneratorNode::OUTPUT_TEXTURE,
          texture_out,
          ColoredShapeRenderNode::INPUT_TEXTURE,
        )
        .unwrap();
      graph
        .connect(
          texture_out,
          ColoredShapeRenderNode::OUTPUT_TEXTURE,
          graph.get_end_node(),
          PresentToScreen::INPUT_TEXTURE_UI,
        )
        .unwrap();
    }
  }
}

struct ColoredShapeRenderNode {
  indices: u16,
  vb: StagedBufferAccessor,
  ib: StagedBufferAccessor,
  pipeline: ResourceRc<RenderPipeline>,
  transform: GenericUniform,
}

impl ColoredShapeRenderNode {
  const INPUT_TEXTURE: usize = 0;
  const OUTPUT_TEXTURE: usize = 0;
}

impl FrameGraphNode for ColoredShapeRenderNode {
  fn execute(
    &self,
    inputs: &[Option<FrameNodeValue>],
    outputs: &mut [Option<FrameNodeValue>],
    encoder: &mut CommandEncoder,
  ) {
    let texture = inputs[Self::INPUT_TEXTURE].as_ref().unwrap();

    let vb = self.vb.get_resources(encoder);
    let ib = self.ib.get_resources(encoder);
    let transform = self.transform.get_resources(encoder);

    let mut rp_builder = RenderPassCommandEncoderBuilder::new("UIRenderPassColoredShape");
    rp_builder.add_color_output(
      &texture.get_sampled_texture().view,
      Vector4::new(0.5, 0.0, 0.0, 0.0),
    );

    let mut rp = encoder.create_render_pass_encoder(rp_builder);
    rp.set_vertex_buffer(vb.clone());
    rp.set_index_buffer(ib.clone(), IndexFormat::Uint16);
    rp.set_bind_group(0, transform.bind_group.clone());
    rp.set_pipeline(self.pipeline.clone());
    rp.render_indexed(0..self.indices as u32);

    outputs[Self::OUTPUT_TEXTURE] = Some(texture.clone());
  }
}

pub struct ShapeManager {
  dirty: AtomicBool,
  colored_shapes: Mutex<Arena<ColoredShape>>,
}

pub type ColoredShapeGeometry = VertexBuffers<Point, u16>;
pub struct ColoredShape {
  color: Vector4<f32>,
  geometry: ColoredShapeGeometry,
}

impl ShapeManager {
  fn new() -> Self {
    ShapeManager {
      colored_shapes: Mutex::new(Arena::new()),
      dirty: AtomicBool::new(false),
    }
  }

  pub fn add_colored_shape(&self, color: Vector4<f32>, geometry: ColoredShapeGeometry) -> Index {
    let mut shapes = self.colored_shapes.lock();
    self.dirty.store(true, Ordering::Relaxed);
    shapes.insert(ColoredShape { color, geometry })
  }
}

lazy_static! {
  pub(crate) static ref SHAPE_MANAGER: ShapeManager = ShapeManager::new();
}
