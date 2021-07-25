use itertools::*;
use lazy_static::lazy_static;
use legion::world::SubWorld;
use legion::IntoQuery;
use moonwave_common::{MetricSpace, Vector4};
use moonwave_core::*;
use moonwave_render::{
  CommandEncoder, FrameGraphNode, FrameNodeValue, RenderPassCommandEncoderBuilder,
};
use moonwave_resources::{
  BindGroup, Buffer, IndexFormat, RenderPipeline, ResourceRc, TextureFormat,
};
use moonwave_shader::ShaderBuildParams;
use moonwave_shader::VertexStruct;
use parking_lot::Mutex;
use rayon::prelude::*;
use std::collections::HashMap;
use std::hash::Hash;
use std::hash::Hasher;
use std::sync::Arc;

use crate::opt::GenericStaticMeshCombiner;
use crate::opt::StaticMeshCombiner;
use crate::opt::StaticMeshCombinerEntry;
use crate::MeshVertexNormal;
use crate::TransformOptimization;
use crate::{
  BoundingShape, BuiltMaterial, Camera, GenericUniform, LightManager, MainCameraTag, Material,
  Mesh, MeshIndex, MeshVertex, Transform,
};

static REGISTERED_SYSTEM: std::sync::Once = std::sync::Once::new();
static PBR_MAIN_COLOR: OnceCell<Arc<TextureGeneratorHost>> = OnceCell::new();
static PBR_MAIN_DEPTH: OnceCell<Arc<TextureGeneratorHost>> = OnceCell::new();

pub struct MeshRenderer {
  vertex_buffer: Option<ResourceRc<Buffer>>,
  indices: u32,
  index_buffer: Option<ResourceRc<Buffer>>,
  static_entry: Option<(StaticRenderGroup, StaticMeshCombinerEntry)>,
  index_format: IndexFormat,
  material: Arc<BuiltMaterial>,
  bindings: Vec<ResourceRc<BindGroup>>,
}

impl MeshRenderer {
  pub fn new<
    T: MeshVertex + MeshVertexNormal + VertexStruct + Send + Sync + 'static,
    I: MeshIndex + Send + Sync + 'static,
  >(
    material: &Material,
    mesh: &Mesh<T, I>,
    bindings: Vec<ResourceRc<BindGroup>>,
    transform: &Transform,
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

    // Build material
    let mut params = ShaderBuildParams::new();
    params.add(ShaderOptionsMeshRenderer {
      no_transform: matches!(transform.get().opt, TransformOptimization::Static),
    });
    let material = material.build(&params);

    // Further processes
    let (vertex_buffer, index_buffer, static_entry) = match transform.get().opt {
      // For static we start merging the to be rendered object into a shared buffer.
      TransformOptimization::Static => {
        // Create group
        let group = StaticRenderGroup {
          material: material.clone(),
          index_size: std::mem::size_of::<I>(),
          bindings: bindings.clone(),
        };

        let mut mesh_groups = MERGED_MESH_GROUPS.lock();
        let entry = if let Some(any_combiner) = mesh_groups.get(&group) {
          let combiner = any_combiner
            .as_any()
            .downcast_ref::<StaticMeshCombiner<T, I>>()
            .unwrap();
          combiner.insert(mesh, transform).unwrap()
        } else {
          // No mesh combiner for this specific group found -> create a new one
          let combiner = StaticMeshCombiner::<T, I>::new(1024, 16, 1024, 10 * 1024);
          let entry = combiner.insert(mesh, transform).unwrap();

          // Store mesh combiner for future objects of same group.
          let boxed = Box::new(combiner);
          mesh_groups.insert(group.clone(), boxed);

          entry
        };

        (None, None, Some((group, entry)))
      }
      // For dynamic we need to create vertex and index buffers on the fly.
      TransformOptimization::Dynamic => (
        Some(mesh.build_vertex_buffer()),
        Some(mesh.build_index_buffer()),
        None,
      ),
    };

    Self {
      vertex_buffer,
      index_buffer,
      static_entry,
      indices: mesh.len_indices() as u32,
      material,
      index_format: I::get_format(),
      bindings,
    }
  }
}

#[derive(Clone)]
struct StaticRenderGroup {
  material: Arc<BuiltMaterial>,
  bindings: Vec<ResourceRc<BindGroup>>,
  index_size: usize,
}

impl Hash for StaticRenderGroup {
  fn hash<H: Hasher>(&self, state: &mut H) {
    // Hashing the material is enough here as the exact same material would never be used for multiple vertex buffers.
    self.material.hash(state);
    self.bindings.hash(state);
    state.write_usize(self.index_size);
  }
}
impl PartialEq for StaticRenderGroup {
  fn eq(&self, other: &Self) -> bool {
    self
      .material
      .vertex_shader
      .eq(&other.material.vertex_shader)
      && self.index_size == other.index_size
  }
}
impl Eq for StaticRenderGroup {}

lazy_static! {
  static ref MERGED_MESH_GROUPS: Mutex<HashMap<StaticRenderGroup, Box<dyn GenericStaticMeshCombiner + Send + Sync + 'static>>> =
    Mutex::new(HashMap::new());
}

#[derive(Hash)]
pub(crate) struct ShaderOptionsMeshRenderer {
  /// Disables transform matrix transformation
  pub(crate) no_transform: bool,
}

#[system]
#[write_component(MeshRenderer)]
#[read_component(Transform)]
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

  // Query light manager.
  let light_manager_uniform = {
    let mut query = <&LightManager>::query();
    let manager = query.iter(world).next().map(|val| val.get_uniform());
    if manager.is_none() {
      return;
    }
    manager.unwrap()
  };

  // Query all meshes
  let mut objs_query = <(&mut MeshRenderer, &Transform, &BoundingShape)>::query();

  // Query all relevant visible meshes and calculate cam distance for later depth based sorting.
  let ready_entities = objs_query
    .par_iter_mut(world)
    // Filter out invisible meshes and calculate their distance to camera.
    .filter_map(|(obj, transform, bshape)| {
      // Remove out of frustum
      if !bshape.visible_in_frustum(&main_cam_frustum) {
        return None;
      }

      // Calculate distance
      let distance = transform.get().position.distance(main_cam_eye).abs();
      Some((obj, transform, distance))
    })
    .collect::<Vec<_>>();

  // Query all static static meshes
  let static_objs = ready_entities
    .iter()
    .filter(|(_, transform, _)| matches!(transform.get().opt, TransformOptimization::Static));

  let static_groups = static_objs
    .group_by(|(obj, _, _)| obj.static_entry.as_ref().unwrap().0.clone())
    .into_iter()
    .map(|(group, entries)| StaticRenderDrawGroup {
      group,
      entries: entries
        .map(|(obj, _, _)| obj.static_entry.as_ref().unwrap().1.clone())
        .collect_vec(),
      system_uniforms: vec![main_cam_uniform.as_generic(), light_manager_uniform.clone()],
    })
    .collect_vec();

  // Query all dynamic meshes and put them into render graph node as dynamic nodes.
  let dyn_objs = ready_entities
    .iter()
    .filter(|(_, transform, _)| matches!(transform.get().opt, TransformOptimization::Dynamic));

  // Build logical grouping by material.
  let material_grouped = dyn_objs.into_group_map_by(|(obj, _, _)| obj.material.clone());

  let render_groups = material_grouped
    .iter()
    .map(|(material, objs)| RenderGroup {
      pipeline: material.pbr_pipeline.clone(),
      objects: objs
        .iter()
        .map(|(obj, transform, _distance)| SingleRenderObject {
          index_format: obj.index_format,
          vertex_buffer: obj.vertex_buffer.clone().unwrap(),
          index_buffer: obj.index_buffer.clone().unwrap(),
          indices: obj.indices,
          uniforms: vec![
            main_cam_uniform.as_generic(),
            transform.uniform.as_ref().unwrap().as_generic(),
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
      dynamic_groups: render_groups,
      static_groups,
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
  dynamic_groups: Vec<RenderGroup>,
  static_groups: Vec<StaticRenderDrawGroup>,
}

struct StaticRenderDrawGroup {
  group: StaticRenderGroup,
  entries: Vec<StaticMeshCombinerEntry>,
  system_uniforms: Vec<GenericUniform>,
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
      .dynamic_groups
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

    // Access uniforms
    let static_uniforms = self
      .static_groups
      .iter()
      .map(|group| {
        group
          .system_uniforms
          .iter()
          .map(|uniform| uniform.get_resources(encoder))
          .collect::<Vec<_>>()
      })
      .collect::<Vec<_>>();

    {
      optick::event!("FrameGraph::PBR::RenderPass");
      let mut rp = encoder.create_render_pass_encoder(rpb);

      // Render static
      {
        optick::event!("FrameGraph::PBR::RenderPassStatic");
        let mesh_combiners = MERGED_MESH_GROUPS.lock();

        for (group_index, group) in self.static_groups.iter().enumerate() {
          // Prepare shared rendering.
          let combiner = mesh_combiners.get(&group.group).unwrap();
          rp.set_pipeline(group.group.material.pbr_pipeline.clone());

          // Set bind groups.
          let uniforms = &static_uniforms[group_index];
          for (index, res) in uniforms.iter().enumerate() {
            rp.set_bind_group(index as u32, res.bind_group.clone());
          }
          for (index, bind_group) in group.group.bindings.iter().enumerate() {
            rp.set_bind_group(index as u32 + uniforms.len() as u32, bind_group.clone());
          }

          // Build optimized draw calls for static meshes.
          combiner.merged_draw(&group.entries, &mut rp);
        }
      }

      for (group_index, group) in self.dynamic_groups.iter().enumerate() {
        rp.set_pipeline(group.pipeline.clone());
        for (object_index, object) in group.objects.iter().enumerate() {
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
