use futures::Future;
use moonwave_common::Vector2;
use moonwave_render::{DeviceHost, FrameGraph};
use std::{
  sync::{Arc, RwLock},
  time::Instant,
};

use shaderc::Compiler;
pub use shaderc::ShaderKind;
use thiserror::Error;
use wgpu::{
  util::DeviceExt, BufferDescriptor, Device, Queue, Surface, SwapChain, SwapChainDescriptor,
  SwapChainError,
};

use crate::{
  execution::Execution, nodes::PresentToScreen, warn, EstimatedExecutionTime, Extension,
  ExtensionHost, GenericIntoActor, World,
};

use moonwave_resources::*;

pub struct Core {
  device: Device,
  queue: Queue,
  swap_chain: SwapChain,
  sc_desc: SwapChainDescriptor,
  surface: Surface,
  pub(crate) execution: Execution,
  resources: ResourceStorage,
  extension_host: RwLock<ExtensionHost>,
  graph: FrameGraph,
  world: Option<Arc<World>>,
  last_frame: Instant,
  pub(crate) arced: Option<Arc<Core>>,
}

impl Core {
  pub(crate) fn new(
    device: Device,
    queue: Queue,
    swap_chain: SwapChain,
    sc_desc: SwapChainDescriptor,
    surface: Surface,
  ) -> Self {
    // Execution
    let execution = Execution::new(0.5);
    execution.start();

    // Build self
    Self {
      last_frame: Instant::now(),
      swap_chain,
      sc_desc,
      device,
      queue,
      surface,
      execution,
      graph: FrameGraph::new(PresentToScreen {}),
      resources: ResourceStorage::new(),
      extension_host: RwLock::new(ExtensionHost::new()),
      world: None,
      arced: None,
    }
  }

  pub(crate) fn setup<T: GenericIntoActor>(&mut self, arced: Arc<Self>, actor: T) {
    self.arced = Some(arced.clone());
    self.world = Some(Arc::new(World::new(arced, actor)));
  }

  pub(crate) fn recreate_swap_chain(&mut self, width: u32, height: u32) {
    self.sc_desc.width = width;
    self.sc_desc.height = height;
    self.swap_chain = self.device.create_swap_chain(&self.surface, &self.sc_desc);
  }

  pub fn get_swap_chain_size(&self) -> Vector2<u32> {
    Vector2::new(self.sc_desc.width, self.sc_desc.height)
  }

  pub(crate) fn frame(&mut self, arced: Arc<Core>) -> Result<(), SwapChainError> {
    // Timing
    let time = Instant::now();
    let duration = time - self.last_frame;
    self.last_frame = time;

    // Next frame.
    let swap_frame = Arc::new(self.swap_chain.get_current_frame()?);

    // Execute extensions
    {
      let ext_host = self.extension_host.read().unwrap();
      ext_host.before_tick(arced.clone());
    }

    // Execute ecs
    {
      optick::event!("Core::frame::ecs_tick");
      self.get_world().tick(duration.as_millis() as u64);
    }

    // Execute graph
    {
      optick::event!("Core::frame::execute_graph");
      self
        .graph
        .execute(swap_frame.clone(), arced.clone(), |fut| {
          Box::pin(arced.schedule_task(TaskKind::RenderGraph, fut))
        });
    }

    {
      optick::event!("Core::frame::swapchain_drop");
      drop(swap_frame);
    }

    Ok(())
  }

  /// Registers a core extension
  pub fn add_extension<T: Extension>(&self, extension: T) {
    let mut host = self.extension_host.write().unwrap();
    host.add(extension);
  }

  /// Returns the ecs systems world container.
  pub fn get_world(&self) -> &Arc<World> {
    self.world.as_ref().unwrap()
  }

  pub fn get_frame_graph(&self) -> &FrameGraph {
    &self.graph
  }

  pub fn get_arced(&self) -> Arc<Core> {
    self.arced.as_ref().unwrap().clone()
  }

  /// Creates a new memory buffer on the GPU and initiales it with the given data.
  pub async fn create_inited_buffer(
    &self,
    data: Box<[u8]>,
    usage: BufferUsage,
    label: Option<&str>,
  ) -> ResourceRc<Buffer> {
    let label = label.map(|l| l.to_string());
    let self_cloned = self.arced.as_ref().unwrap().clone();
    self
      .schedule_weighted_task(
        TaskKind::Background,
        async move {
          optick::event!("Core::create_inited_buffer");

          // Create inited data.
          let buffer = self_cloned
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
              label: label.as_deref(),
              usage: wgpu::BufferUsage::from_bits(usage.bits()).unwrap(),
              contents: &data,
            });

          // Create proxy
          self_cloned.resources.create_proxy(buffer)
        },
        EstimatedExecutionTime::FractionOfFrame(1),
      )
      .await
  }

  /// Creates a new memory buffer on the GPU.
  pub async fn create_buffer(
    &self,
    size: u64,
    mapped_at_creation: bool,
    usage: BufferUsage,
    label: Option<&str>,
  ) -> ResourceRc<Buffer> {
    // Prepare
    let label = label.map(|l| l.to_string());

    // Create raw device buffer.
    let self_cloned = self.arced.as_ref().unwrap().clone();
    self
      .schedule_weighted_task(
        TaskKind::Background,
        async move {
          optick::event!("Core::create_buffer");

          let buffer = self_cloned.device.create_buffer(&BufferDescriptor {
            label: label.as_deref(),
            mapped_at_creation,
            size,
            usage: wgpu::BufferUsage::from_bits(usage.bits()).unwrap(),
          });

          // Create proxy
          self_cloned.resources.create_proxy(buffer)
        },
        EstimatedExecutionTime::FractionOfFrame(1),
      )
      .await
  }

  /// Creates a new empty texture
  pub async fn create_texture(
    &self,
    label: Option<&str>,
    usage: TextureUsage,
    format: TextureFormat,
    size: Vector2<u32>,
    mips: u32,
  ) -> ResourceRc<Texture> {
    // Prepare
    let label = label.map(|l| l.to_string());

    // Create raw device buffer.
    let self_cloned = self.arced.as_ref().unwrap().clone();
    self
      .schedule_weighted_task(
        TaskKind::Background,
        async move {
          optick::event!("Core::create_texture");
          let raw = self_cloned.device.create_texture(&wgpu::TextureDescriptor {
            label: label.as_deref(),
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            size: wgpu::Extent3d {
              width: size.x,
              height: size.y,
              depth: 1,
            },
            usage,
            format,
          });

          // Create proxy
          self_cloned.resources.create_proxy(raw)
        },
        EstimatedExecutionTime::FractionOfFrame(1),
      )
      .await
  }

  /// Creates a new texture view.
  pub async fn create_texture_view(&self, texture: ResourceRc<Texture>) -> ResourceRc<TextureView> {
    // Create raw device buffer.
    let self_cloned = self.arced.as_ref().unwrap().clone();
    self
      .schedule_weighted_task(
        TaskKind::Background,
        async move {
          optick::event!("Core::create_texture_view");
          let raw = texture
            .get_raw()
            .create_view(&wgpu::TextureViewDescriptor::default());

          // Create proxy
          self_cloned.resources.create_proxy(raw)
        },
        EstimatedExecutionTime::FractionOfFrame(1),
      )
      .await
  }

  /// Creates a new bind group layout.
  pub async fn create_bind_group_layout(
    &self,
    desc: BindGroupLayoutDescriptor,
  ) -> ResourceRc<BindGroupLayout> {
    let self_cloned = self.arced.as_ref().unwrap().clone();

    self
      .schedule_task(TaskKind::Background, async move {
        optick::event!("Core::create_bind_group_layout");

        let entries = desc
          .entries
          .iter()
          .map(|entry| wgpu::BindGroupLayoutEntry {
            binding: entry.binding,
            count: None,
            visibility: wgpu::ShaderStage::all(),
            ty: match entry.ty {
              BindGroupLayoutEntryType::UniformBuffer => wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
              },
            },
          })
          .collect::<Vec<_>>();

        let raw = self_cloned
          .device
          .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &entries,
          });

        self_cloned.resources.create_proxy(raw)
      })
      .await
  }

  /// Creates a new pipeline layout.
  pub async fn create_pipeline_layout(
    &self,
    desc: PipelineLayoutDescriptor,
  ) -> ResourceRc<PipelineLayout> {
    let self_cloned = self.arced.as_ref().unwrap().clone();

    self
      .schedule_task(TaskKind::Background, async move {
        optick::event!("Core::create_pipeline_layout");

        let bindings = desc
          .bindings
          .iter()
          .map(|binding| binding.get_raw())
          .collect::<Vec<_>>();

        let raw = self_cloned
          .device
          .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &bindings
              .iter()
              .map(|binding| &**binding)
              .collect::<Vec<_>>(),
            push_constant_ranges: &[],
          });

        self_cloned.resources.create_proxy(raw)
      })
      .await
  }

  /// Creates a new bind group.
  pub async fn create_bind_group(&self, desc: BindGroupDescriptor) -> ResourceRc<BindGroup> {
    let self_cloned = self.arced.as_ref().unwrap().clone();

    self
      .schedule_task(TaskKind::Background, async move {
        optick::event!("Core::create_bind_group");

        // Get locks on all related resources.
        let entries = desc
          .entries
          .iter()
          .map(|(binding, res)| (*binding, res.read()))
          .collect::<Vec<_>>();

        // Create bind group entry
        let wgpu_entries = entries.iter().map(|(binding, entry)| {
          (
            *binding,
            match entry {
              UnlockedBindGroupEntry::Buffer(buffer) => wgpu::BindingResource::Buffer {
                buffer: &*buffer,
                offset: 0,
                size: None,
              },
            },
          )
        });

        let raw = self_cloned
          .device
          .create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &*desc.layout.get_raw(),
            entries: &wgpu_entries
              .into_iter()
              .map(|(binding, resource)| wgpu::BindGroupEntry { binding, resource })
              .collect::<Vec<_>>(),
          });

        self_cloned.resources.create_proxy(raw)
      })
      .await
  }

  /// Creates a new bind group.
  pub async fn create_render_pipeline(
    &self,
    desc: RenderPipelineDescriptor,
  ) -> ResourceRc<RenderPipeline> {
    let self_cloned = self.arced.as_ref().unwrap().clone();

    self
      .schedule_task(TaskKind::Background, async move {
        optick::event!("Core::create_render_pipeline");

        let vs = desc.vertex_shader.get_raw();
        let fs = desc.fragment_shader.get_raw();

        let raw = self_cloned
          .device
          .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&*desc.layout.get_raw()),
            multisample: wgpu::MultisampleState::default(),
            vertex: wgpu::VertexState {
              module: &*vs,
              entry_point: "main",
              buffers: &[wgpu::VertexBufferLayout {
                array_stride: desc.vertex_desc.stride,
                step_mode: wgpu::InputStepMode::Vertex,
                attributes: &desc
                  .vertex_desc
                  .attributes
                  .iter()
                  .map(|attr| wgpu::VertexAttribute {
                    shader_location: attr.location as u32,
                    offset: attr.offset,
                    format: attr.format.to_wgpu(),
                  })
                  .collect::<Vec<_>>(),
              }],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: desc.depth.map(|depth| wgpu::DepthStencilState {
              bias: wgpu::DepthBiasState::default(),
              stencil: wgpu::StencilState::default(),
              format: depth,
              clamp_depth: false,
              depth_compare: wgpu::CompareFunction::GreaterEqual,
              depth_write_enabled: true,
            }),
            fragment: Some(wgpu::FragmentState {
              module: &*fs,
              entry_point: "main",
              targets: &desc
                .outputs
                .iter()
                .map(|output| wgpu::ColorTargetState {
                  format: output.format,
                  alpha_blend: wgpu::BlendState::default(),
                  color_blend: wgpu::BlendState::default(),
                  write_mask: wgpu::ColorWrite::all(),
                })
                .collect::<Vec<_>>(),
            }),
          });

        self_cloned.resources.create_proxy(raw)
      })
      .await
  }

  /// Creates a raw shader from vulkan compatible glsl.
  pub async fn create_shader_from_glsl(
    &self,
    source: &str,
    name: &str,
    kind: ShaderKind,
  ) -> Result<ResourceRc<Shader>, ShaderError> {
    let source_string = source.to_string();
    let name_string = name.to_string();
    let label_string = name.to_string();
    let self_cloned = self.arced.as_ref().unwrap().clone();

    self
      .schedule_weighted_task(
        TaskKind::Background,
        async move {
          optick::event!("Core::create_shader");
          let mut compiler = Compiler::new().unwrap();

          // Compile to spir-v
          let spirv = compiler
            .compile_into_spirv(
              source_string.as_str(),
              kind,
              name_string.as_str(),
              "main",
              None,
            )
            .map_err(|err| ShaderError::SpirVCompilationFailed(err.to_string()))?;

          if spirv.get_num_warnings() > 0 {
            warn!(
              "Shader compilation warning: {}",
              spirv.get_warning_messages()
            );
          }
          let spirv = spirv.as_binary_u8().to_vec();

          // Create raw resource
          let module = {
            self_cloned
              .device
              .create_shader_module(&wgpu::ShaderModuleDescriptor {
                label: Some(label_string.as_str()),
                source: wgpu::util::make_spirv(&spirv),
                flags: wgpu::ShaderFlags::VALIDATION,
              })
          };

          // Create proxy
          Ok(self_cloned.resources.create_proxy(module))
        },
        EstimatedExecutionTime::FractionOfFrame(1),
      )
      .await
  }

  pub fn schedule_task<F: Future + Send + Sync + 'static>(
    &self,
    kind: TaskKind,
    future: F,
  ) -> impl Future<Output = F::Output> + Send + Sync + 'static
  where
    F::Output: Send + Sync + 'static,
  {
    self.schedule_weighted_task(kind, future, EstimatedExecutionTime::Unspecified)
  }

  pub fn schedule_weighted_task<F: Future + Send + Sync + 'static>(
    &self,
    kind: TaskKind,
    future: F,
    estimation: EstimatedExecutionTime,
  ) -> std::pin::Pin<Box<dyn Future<Output = F::Output> + Send + Sync + 'static>>
  where
    F::Output: Send + Sync + 'static,
  {
    match kind {
      TaskKind::Background => Box::pin(self.execution.add_background_task(future, estimation)),
      TaskKind::ECS => Box::pin(self.execution.add_ecs_task(future, estimation)),
      TaskKind::RenderGraph => Box::pin(self.execution.add_graph_task(future, estimation)),
      TaskKind::Main => Box::pin(self.execution.add_main_task(future)),
    }
  }
}

#[derive(Error, Debug)]
pub enum ShaderError {
  #[error("Failed to compile glsl shader to spir-v: {0}")]
  SpirVCompilationFailed(String),
}

pub enum TaskKind {
  Main,
  Background,
  ECS,
  RenderGraph,
}

impl DeviceHost for Core {
  fn get_device(&self) -> &wgpu::Device {
    &self.device
  }
  fn get_queue(&self) -> &wgpu::Queue {
    &self.queue
  }
}

pub trait BindGroupLayoutSingleton {
  fn get_bind_group_lazy(core: &Core) -> ResourceRc<BindGroupLayout>;
}
