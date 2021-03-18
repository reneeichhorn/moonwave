use legion::world::SubWorld;
use legion::IntoQuery;
use moonwave_common::Vector3;
use moonwave_core::*;
use moonwave_render::{
  CommandEncoder, FrameGraphNode, FrameNodeValue, RenderPassCommandEncoderBuilder,
};
use moonwave_resources::{
  BindGroup, BindGroupDescriptor, BindGroupLayout, Buffer, IndexFormat, RenderPipeline,
  RenderPipelineDescriptor, ResourceRc, TextureFormat, VertexBuffer,
};
use moonwave_shader::VertexStruct;
use parking_lot::Mutex;
use std::sync::Arc;

use crate::{
  BuiltMaterial, Camera, CameraUniform, DynamicUniformNode, GenericUniform, MainCameraTag,
  Material, Mesh, MeshIndex, MeshVertex, Model,
};

static REGISTERED_SYSTEM: std::sync::Once = std::sync::Once::new();
static PBR_MAIN_COLOR: OnceCell<Arc<TextureGeneratorHost>> = OnceCell::new();
static PBR_MAIN_DEPTH: OnceCell<Arc<TextureGeneratorHost>> = OnceCell::new();

pub struct StaticMeshRenderer {
  vertex_buffer_desc: VertexBuffer,
  vertex_buffer: ResourceRc<Buffer>,
  indices: u32,
  index_buffer: ResourceRc<Buffer>,
  index_format: IndexFormat,
  material: Arc<BuiltMaterial>,
  cache: Arc<Mutex<StaticMeshRendererCache>>,
  bindings: Vec<ResourceRc<BindGroup>>,
}

impl StaticMeshRenderer {
  pub fn new<T: MeshVertex + VertexStruct, I: MeshIndex>(
    material: &Material,
    mesh: &Mesh<T, I>,
    bindings: Vec<ResourceRc<BindGroup>>,
  ) -> Self {
    REGISTERED_SYSTEM.call_once(|| {
      // Create texture nodes.
      let color = TextureGeneratorHost::new(TextureSize::FullScreen, TextureFormat::Bgra8Unorm);
      let depth = TextureGeneratorHost::new(TextureSize::FullScreen, TextureFormat::Depth32Float);

      PBR_MAIN_COLOR.set(color).ok().unwrap();
      PBR_MAIN_DEPTH.set(depth).ok().unwrap();

      // Add system if not added yet.
      Core::get_instance()
        .get_world()
        .add_system_to_stage(CreatePBRFrameGraphSystem, SystemStage::Rendering);
    });

    // Build buffers.
    let vertex_buffer = mesh.build_vertex_buffer();
    let index_buffer = mesh.build_index_buffer();

    // Build material
    let material = material.build();

    Self {
      vertex_buffer_desc: T::generate_buffer(),
      vertex_buffer,
      index_buffer,
      indices: mesh.len_indices() as u32,
      material,
      index_format: I::get_format(),
      bindings,
      cache: Arc::new(Mutex::new(StaticMeshRendererCache::Empty)),
    }
  }
}
enum StaticMeshRendererCache {
  Empty,
  Creating,
  Created(ResourceRc<RenderPipeline>),
}

#[system]
#[write_component(StaticMeshRenderer)]
#[read_component(Model)]
#[read_component(MainCameraTag)]
#[read_component(Camera)]
pub fn create_pbr_frame_graph(world: &mut SubWorld) {
  optick::event!("create_pbr_frame_graph");

  // Get main camera and its frame node.
  let main_cam_uniform = {
    let mut main_cam_query = <(&Camera, &MainCameraTag)>::query();
    let (main_cam, _) = main_cam_query
      .iter(world)
      .next()
      .unwrap_or_else(|| panic!("No main camera found in scene"));
    main_cam.uniform.clone()
  };

  // Query all static meshes
  let mut static_objs_query = <(&mut StaticMeshRenderer, &Model)>::query();
  let single_render_objects = static_objs_query
    .iter_mut(world)
    .filter_map(|(obj, model)| {
      // Build cache
      let pipeline = {
        let mut cache_guard = obj.cache.lock();
        match &*cache_guard {
          // Cache is already being created, renderer needs to wait therefore.
          StaticMeshRendererCache::Creating => {
            return None;
          }
          // Create cache for the first time.
          StaticMeshRendererCache::Empty => {
            *cache_guard = StaticMeshRendererCache::Creating;
            let cache = obj.cache.clone();

            let vs = obj.material.vertex_shader.clone();
            let fs = obj.material.fragment_shader.clone();
            let layout = obj.material.layout.clone();
            let vb = obj.vertex_buffer_desc.clone();

            Core::get_instance().spawn_background_task(move || {
              let pipeline = Core::get_instance().create_render_pipeline(
                RenderPipelineDescriptor::new(layout, vb, vs, fs)
                  .add_depth(TextureFormat::Depth32Float)
                  .add_color_output(TextureFormat::Bgra8Unorm),
              );
              *cache.lock() = StaticMeshRendererCache::Created(pipeline);
            });
            return None;
          }
          // Cache is ready for this object and we can continue.
          StaticMeshRendererCache::Created(pipeline) => pipeline.clone(),
        }
      };
      Some(SingleRenderObject {
        pipeline,
        index_format: obj.index_format,
        vertex_buffer: obj.vertex_buffer.clone(),
        index_buffer: obj.index_buffer.clone(),
        indices: obj.indices,
        uniforms: vec![main_cam_uniform.as_generic(), model.uniform.as_generic()],
        bindings: obj.bindings.clone(),
      })
    })
    .collect::<Vec<_>>();

  // Build frame graph.
  let frame_graph = Core::get_instance().get_frame_graph();
  let pbr_main_color = frame_graph.add_node(
    PBR_MAIN_COLOR.get().unwrap().create_node(),
    "pbr_main_color",
  );
  let pbr_main_depth = frame_graph.add_node(
    PBR_MAIN_DEPTH.get().unwrap().create_node(),
    "pbr_main_depth",
  );
  let pbr_node = frame_graph.add_node(
    PBRRenderGraphNode {
      objects: single_render_objects,
    },
    "pbr_main_node",
  );
  frame_graph
    .connect(
      pbr_node,
      PBRRenderGraphNode::OUTPUT_COLOR,
      frame_graph.get_end_node(),
      PresentToScreen::INPUT_TEXTURE,
    )
    .unwrap();
  frame_graph
    .connect(
      pbr_main_color,
      TextureGeneratorNode::OUTPUT_TEXTURE,
      pbr_node,
      PBRRenderGraphNode::INPUT_COLOR,
    )
    .unwrap();
  frame_graph
    .connect(
      pbr_main_depth,
      TextureGeneratorNode::OUTPUT_TEXTURE,
      pbr_node,
      PBRRenderGraphNode::INPUT_DEPTH,
    )
    .unwrap();
}
struct CreatePBRFrameGraphSystem;
impl SystemFactory for CreatePBRFrameGraphSystem {
  fn create_system(&self) -> WrappedSystem {
    WrappedSystem(Box::new(create_pbr_frame_graph_system()))
  }
}

struct PBRRenderGraphNode {
  objects: Vec<SingleRenderObject>,
}
struct SingleRenderObject {
  pipeline: ResourceRc<RenderPipeline>,
  vertex_buffer: ResourceRc<Buffer>,
  index_buffer: ResourceRc<Buffer>,
  index_format: IndexFormat,
  uniforms: Vec<GenericUniform>,
  bindings: Vec<ResourceRc<BindGroup>>,
  indices: u32,
}
impl PBRRenderGraphNode {
  pub const INPUT_COLOR: usize = 0;
  pub const INPUT_DEPTH: usize = 1;
  pub const OUTPUT_COLOR: usize = 0;
}

impl FrameGraphNode for PBRRenderGraphNode {
  fn execute(
    &self,
    inputs: &[Option<FrameNodeValue>],
    _outputs: &mut [Option<FrameNodeValue>],
    encoder: &mut CommandEncoder,
  ) {
    optick::event!("FrameGraph::PBR");

    // Prepare uniforms
    let uniforms = self
      .objects
      .iter()
      .flat_map(|object| object.uniforms.iter())
      .map(|uniform| uniform.get_resources(encoder).bind_group.clone())
      .collect::<Vec<_>>();

    // Create render pass.
    let mut rpb = RenderPassCommandEncoderBuilder::new("pbr_rp");
    rpb.add_color_output(
      &inputs[Self::INPUT_COLOR]
        .as_ref()
        .unwrap()
        .get_sampled_texture()
        .view,
      Vector3::new(1.0, 1.0, 1.0),
    );
    rpb.add_depth(
      &inputs[Self::INPUT_DEPTH]
        .as_ref()
        .unwrap()
        .get_sampled_texture()
        .view,
    );
    let mut rp = encoder.create_render_pass_encoder(rpb);

    let mut uniform_index = 0;
    for object in &self.objects {
      // Do the rendering in order.
      rp.set_vertex_buffer(object.vertex_buffer.clone());
      rp.set_index_buffer(object.index_buffer.clone(), object.index_format);
      rp.set_pipeline(object.pipeline.clone());
      for (index, _uniform) in object.uniforms.iter().enumerate() {
        rp.set_bind_group(index as u32, uniforms[uniform_index].clone());
        uniform_index += 1;
      }
      for (index, bind_group) in object.bindings.iter().enumerate() {
        rp.set_bind_group(
          object.uniforms.len() as u32 + index as u32,
          bind_group.clone(),
        );
      }
      rp.render_indexed(0..object.indices);
    }
  }
}
