use legion::world::SubWorld;
use legion::IntoQuery;
use moonwave_common::Vector3;
use moonwave_core::*;
use moonwave_render::{
  CommandEncoder, FrameGraphNode, FrameNodeValue, RenderPassCommandEncoderBuilder,
};
use moonwave_resources::{
  BindGroupLayout, Buffer, IndexFormat, RenderPipeline, RenderPipelineDescriptor, ResourceRc,
  TextureFormat, VertexBuffer,
};
use moonwave_shader::VertexStruct;
use parking_lot::Mutex;
use std::sync::Arc;

use crate::{
  BuiltMaterial, CameraActor, CameraUniform, DynamicUniformNode, MainCameraTag, Material, Mesh,
  MeshIndex, MeshVertex, Model,
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
  bind_groups: Vec<ResourceRc<BindGroupLayout>>,
  material: BuiltMaterial,
  cache: Arc<Mutex<StaticMeshRendererCache>>,
}

impl StaticMeshRenderer {
  pub async fn new<T: MeshVertex + VertexStruct, I: MeshIndex>(
    core: &Core,
    material: &mut Material<T>,
    mesh: &Mesh<T, I>,
  ) -> Self {
    REGISTERED_SYSTEM.call_once(|| {
      // Add system if not added yet.
      core
        .get_world()
        .add_system_to_stage(CreatePBRFrameGraphSystem, SystemStage::Rendering);

      // Create texture nodes.
      let color = block_on(TextureGeneratorHost::new(
        core.get_arced(),
        TextureSize::FullScreen,
        TextureFormat::Bgra8Unorm,
      ));
      let depth = block_on(TextureGeneratorHost::new(
        core.get_arced(),
        TextureSize::FullScreen,
        TextureFormat::Depth32Float,
      ));
      PBR_MAIN_COLOR.set(color).ok().unwrap();
      PBR_MAIN_DEPTH.set(depth).ok().unwrap();
    });

    // Build buffers.
    let vertex_buffer = mesh.build_vertex_buffer(core).await;
    let index_buffer = mesh.build_index_buffer(core).await;

    // Build material
    let material = material.build(core).await;

    Self {
      vertex_buffer_desc: T::generate_buffer(),
      vertex_buffer,
      index_buffer,
      indices: mesh.len_indices() as u32,
      material,
      index_format: I::get_format(),
      cache: Arc::new(Mutex::new(StaticMeshRendererCache::Empty)),
      bind_groups: Vec::new(),
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
#[read_component(CameraActor)]
pub fn create_pbr_frame_graph(world: &mut SubWorld) {
  // Get main camera and its frame node.
  let (core, main_cam_node) = {
    let mut main_cam_query = <(&CameraActor, &MainCameraTag)>::query();
    let (main_cam, _) = main_cam_query.iter(world).next().unwrap();
    let core = main_cam.get_ext().core.clone();
    let cam = main_cam.uniform.lazy_get_frame_node(&*core);
    (core, cam)
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
            let core_cloned = core.clone();
            let cache = obj.cache.clone();

            let vs = obj.material.vertex_shader.clone();
            let fs = obj.material.fragment_shader.clone();
            let layout = obj.material.layout.clone();
            let vb = obj.vertex_buffer_desc.clone();

            let _ = core.schedule_task(TaskKind::Background, async move {
              let pipeline = core_cloned
                .create_render_pipeline(
                  RenderPipelineDescriptor::new(layout, vb, vs, fs)
                    .add_depth(TextureFormat::Depth32Float)
                    .add_color_output(TextureFormat::Bgra8Unorm),
                )
                .await;
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
        bind_group_indices: vec![0, 1],
      })
    })
    .collect::<Vec<_>>();

  // Build frame graph.
  let frame_graph = core.get_frame_graph();
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
      main_cam_node,
      DynamicUniformNode::<CameraUniform>::OUTPUT_BIND_GROUP,
      pbr_node,
      PBRRenderGraphNode::INPUT_BIND_GROUPS,
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
  bind_group_indices: Vec<u8>,
  indices: u32,
}
impl PBRRenderGraphNode {
  pub const INPUT_COLOR: usize = 0;
  pub const INPUT_DEPTH: usize = 1;
  pub const INPUT_BIND_GROUPS: usize = 2;
  pub const OUTPUT_COLOR: usize = 0;
}

impl FrameGraphNode for PBRRenderGraphNode {
  fn execute(
    &self,
    inputs: &[Option<FrameNodeValue>],
    _outputs: &mut [Option<FrameNodeValue>],
    encoder: &mut CommandEncoder,
  ) {
    // Create render pass.
    let mut rpb = RenderPassCommandEncoderBuilder::new("pbr_rp");
    rpb.add_color_output(
      inputs[Self::INPUT_COLOR]
        .as_ref()
        .unwrap()
        .get_texture_view(),
      Vector3::new(1.0, 1.0, 1.0),
    );
    rpb.add_depth(
      inputs[Self::INPUT_DEPTH]
        .as_ref()
        .unwrap()
        .get_texture_view(),
    );
    let mut rp = encoder.create_render_pass_encoder(rpb);

    for object in &self.objects {
      // Get all bind groups needed for this call.
      let bind_groups = object.bind_group_indices.iter().map(|index| {
        inputs[Self::INPUT_BIND_GROUPS + *index as usize]
          .as_ref()
          .unwrap()
          .get_bind_group()
      });

      // Do the rendering in order.
      rp.set_vertex_buffer(object.vertex_buffer.clone());
      rp.set_index_buffer(object.index_buffer.clone(), object.index_format);
      rp.set_pipeline(object.pipeline.clone());
      for (index, g) in bind_groups.enumerate() {
        rp.set_bind_group(index as u32, g.clone());
      }
      rp.render_indexed(0..object.indices);
    }
  }
}
