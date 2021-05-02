use itertools::*;
use legion::world::SubWorld;
use legion::IntoQuery;
use moonwave_common::{MetricSpace, Vector4};
use moonwave_core::*;
use moonwave_render::{
  CommandEncoder, FrameGraphNode, FrameNodeValue, RenderPassCommandEncoderBuilder,
};
use moonwave_resources::{
  BindGroup, Buffer, IndexFormat, RenderPipeline, ResourceRc, TextureFormat, VertexBuffer,
};
use moonwave_shader::VertexStruct;
use rayon::prelude::*;
use std::sync::Arc;

use crate::{
  BoundingShape, BuiltMaterial, Camera, GenericUniform, LightManager, MainCameraTag, Material,
  Mesh, MeshIndex, MeshVertex, Model,
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
      let color = TextureGeneratorHost::new(TextureSize::FullScreen, TextureFormat::Bgra8UnormSrgb);
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
    }
  }
}

#[system]
#[write_component(StaticMeshRenderer)]
#[read_component(Model)]
#[read_component(MainCameraTag)]
#[read_component(Camera)]
#[read_component(BoundingShape)]
#[read_component(LightManager)]
pub fn create_pbr_frame_graph(world: &mut SubWorld) {
  optick::event!("create_pbr_frame_graph");

  // Get main camera and its frame node.
  let mut main_cam_frustum = [Vector4::<f32>::new(0.0, 0.0, 0.0, 0.0); 6];
  let (main_cam_uniform, main_cam_eye) = {
    let mut main_cam_query = <(&Camera, &MainCameraTag)>::query();
    let main_cam = main_cam_query.iter(world).next();
    if main_cam.is_none() {
      return;
    }
    let (main_cam, _) = main_cam.unwrap();
    main_cam.calculate_frustum_planes(&mut main_cam_frustum);
    (main_cam.uniform.clone(), main_cam.position)
  };

  // Query light manayer.
  let light_manager_uniform = {
    let mut query = <&LightManager>::query();
    let manager = query.iter(world).next().map(|val| val.get_uniform());
    if manager.is_none() {
      return;
    }
    manager.unwrap()
  };

  // Query all static meshes
  let mut static_objs_query = <(&mut StaticMeshRenderer, &Model, &BoundingShape)>::query();

  // Query all relevant visible meshes and calculate cam distance for later depth based sorting.
  let ready_entities = static_objs_query
    .par_iter_mut(world)
    .filter_map(|(obj, model, bshape)| {
      // Remove out of frustum
      if !bshape.visible_in_frustum(&main_cam_frustum) {
        return None;
      }

      // Calculate distance
      let distance = model.get().position.distance(main_cam_eye).abs();
      Some((obj, model, distance))
    })
    .collect::<Vec<_>>();

  // Build logical grouping by material.
  let material_grouped = ready_entities
    .iter()
    .into_group_map_by(|(obj, _, _)| obj.material.clone());

  let render_groups = material_grouped
    .iter()
    .map(|(material, objs)| RenderGroup {
      pipeline: material.pbr_pipeline.clone(),
      objects: objs
        .iter()
        .map(|(obj, model, _distance)| SingleRenderObject {
          index_format: obj.index_format,
          vertex_buffer: obj.vertex_buffer.clone(),
          index_buffer: obj.index_buffer.clone(),
          indices: obj.indices,
          uniforms: vec![
            main_cam_uniform.as_generic(),
            model.uniform.as_generic(),
            light_manager_uniform.clone(),
          ],
          bindings: obj.bindings.clone(),
        })
        .collect::<Vec<_>>(),
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
      groups: render_groups,
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
  groups: Vec<RenderGroup>,
}
struct RenderGroup {
  pipeline: ResourceRc<RenderPipeline>,
  objects: Vec<SingleRenderObject>,
}

struct SingleRenderObject {
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

    // Access uniforms
    let uniforms = self
      .groups
      .iter()
      .map(|group| {
        group
          .objects
          .iter()
          .map(|obj| {
            obj
              .uniforms
              .iter()
              .map(|uniform| uniform.get_resources(encoder))
              .collect::<Vec<_>>()
          })
          .collect::<Vec<_>>()
      })
      .collect::<Vec<_>>();

    // Create render pass.
    let mut rpb = RenderPassCommandEncoderBuilder::new("pbr_rp");
    rpb.add_color_output(
      &inputs[Self::INPUT_COLOR]
        .as_ref()
        .unwrap()
        .get_sampled_texture()
        .view,
      Vector4::new(1.0, 1.0, 1.0, 1.0),
    );
    rpb.add_depth(
      &inputs[Self::INPUT_DEPTH]
        .as_ref()
        .unwrap()
        .get_sampled_texture()
        .view,
    );

    {
      optick::event!("FrameGraph::PBR::RenderPass");
      let mut rp = encoder.create_render_pass_encoder(rpb);

      for (group_index, group) in self.groups.iter().enumerate() {
        rp.set_pipeline(group.pipeline.clone());
        for (object_index, object) in group.objects.iter().enumerate() {
          optick::event!("FrameGraph::PBR::DrawCall");

          // Do the rendering in order.
          rp.set_vertex_buffer(object.vertex_buffer.clone());
          rp.set_index_buffer(object.index_buffer.clone(), object.index_format);
          for (index, _uniform) in object.uniforms.iter().enumerate() {
            rp.set_bind_group(
              index as u32,
              uniforms[group_index][object_index][index]
                .bind_group
                .clone(),
            );
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
  }
}
