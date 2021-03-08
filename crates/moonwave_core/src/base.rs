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
  execution::Execution, warn, Extension, ExtensionHost, PresentToScreen, ServiceLocator, World,
};

use moonwave_resources::*;

static mut CORE: Option<Core> = None;

pub struct Core {
  device: Device,
  queue: Queue,
  swap_chain: SwapChain,
  sc_desc: SwapChainDescriptor,
  surface: Surface,
  resources: ResourceStorage,
  extension_host: RwLock<ExtensionHost>,
  graph: FrameGraph,
  world: World,
  last_frame: Instant,
  service_locator: ServiceLocator,
  execution: Execution,
}

impl Core {
  fn new(
    device: Device,
    queue: Queue,
    swap_chain: SwapChain,
    sc_desc: SwapChainDescriptor,
    surface: Surface,
  ) -> Self {
    Self {
      last_frame: Instant::now(),
      swap_chain,
      sc_desc,
      device,
      queue,
      surface,
      graph: FrameGraph::new(PresentToScreen {}),
      resources: ResourceStorage::new(),
      extension_host: RwLock::new(ExtensionHost::new()),
      service_locator: ServiceLocator::new(),
      execution: Execution::new(6),
      world: World::new(),
    }
  }

  pub(crate) fn initialize(
    device: Device,
    queue: Queue,
    swap_chain: SwapChain,
    sc_desc: SwapChainDescriptor,
    surface: Surface,
  ) {
    unsafe {
      CORE = Some(Core::new(device, queue, swap_chain, sc_desc, surface));
    }
  }

  #[inline]
  pub fn get_instance() -> &'static Core {
    // Only "unsafe" during first initialization.
    // But since initialization takes places before all other usages it's safe.
    unsafe { CORE.as_ref().unwrap() }
  }

  #[inline]
  pub(crate) fn get_instance_mut_unstable() -> &'static mut Core {
    // Only safe when frame graph, world and gpu resources are not in access anymore.
    unsafe { CORE.as_mut().unwrap() }
  }

  pub(crate) fn recreate_swap_chain(&mut self, width: u32, height: u32) {
    self.sc_desc.width = width;
    self.sc_desc.height = height;
    self.swap_chain = self.device.create_swap_chain(&self.surface, &self.sc_desc);
  }

  pub fn get_swap_chain_size(&self) -> Vector2<u32> {
    Vector2::new(self.sc_desc.width, self.sc_desc.height)
  }

  pub(crate) fn frame(&mut self) -> Result<(), SwapChainError> {
    // Timing
    let time = Instant::now();
    let duration = time - self.last_frame;
    self.last_frame = time;

    // Next frame.
    let swap_frame = Arc::new(self.swap_chain.get_current_frame()?);

    // Execute extensions
    /*
    {
      optick::event!("Core::extensions::before_tick");
      let ext_host = self.extension_host.read().unwrap();
      ext_host.before_tick(arced.clone());
    }
    */

    // Execute ecs
    {
      optick::event!("Core::frame::ecs_tick");
      self.world.tick(
        duration.as_micros() as u64,
        self.execution.get_frame_thread_pool(),
      );
    }

    // Execute graph
    {
      optick::event!("Core::frame::execute_graph");
      self.graph.execute(
        swap_frame.clone(),
        Core::get_instance(),
        self.execution.get_frame_thread_pool(),
      );
    }

    {
      optick::event!("Core::frame::swapchain_drop");
      assert_eq!(
        1,
        Arc::strong_count(&swap_frame),
        "Reference to Swapchain frame has not been dropped in frame graph"
      );
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
  #[inline]
  pub fn get_world(&self) -> &World {
    &self.world
  }

  /// Returns the ecs systems world container.
  pub fn get_world_mut(&mut self) -> &mut World {
    &mut self.world
  }

  /// Spawns a background task without waiting for it.
  pub fn spawn_background_task<OP>(&self, op: OP)
  where
    OP: FnOnce() + Send + 'static,
  {
    self.execution.get_background_thread_pool().spawn(op);
  }

  /// Install a task into the background thread pool and returns when its done.
  pub fn install_background_task<OP, R>(&self, op: OP) -> R
  where
    OP: FnOnce() -> R + Send,
    R: Send,
  {
    self.execution.get_background_thread_pool().install(op)
  }

  #[inline]
  pub fn get_frame_graph(&self) -> &FrameGraph {
    &self.graph
  }

  #[inline]
  pub fn get_service_locator(&self) -> &ServiceLocator {
    &self.service_locator
  }

  /// Creates a new memory buffer on the GPU and initiales it with the given data.
  pub fn create_inited_buffer(
    &self,
    data: Box<[u8]>,
    usage: BufferUsage,
    label: Option<&str>,
  ) -> ResourceRc<Buffer> {
    optick::event!("Core::create_inited_buffer");

    // Create inited data.
    let buffer = self
      .device
      .create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label,
        usage: wgpu::BufferUsage::from_bits(usage.bits()).unwrap(),
        contents: &data,
      });

    // Create proxy
    self.resources.create_proxy(buffer)
  }

  /// Creates a new memory buffer on the GPU.
  pub fn create_buffer(
    &self,
    size: u64,
    mapped_at_creation: bool,
    usage: BufferUsage,
    label: Option<&str>,
  ) -> ResourceRc<Buffer> {
    // Create raw device buffer.
    optick::event!("Core::create_buffer");

    let buffer = self.device.create_buffer(&BufferDescriptor {
      label,
      mapped_at_creation,
      size,
      usage: wgpu::BufferUsage::from_bits(usage.bits()).unwrap(),
    });

    // Create proxy
    self.resources.create_proxy(buffer)
  }

  /// Creates a new empty texture
  pub fn create_texture(
    &self,
    label: Option<&str>,
    usage: TextureUsage,
    format: TextureFormat,
    size: Vector2<u32>,
    mips: u32,
  ) -> ResourceRc<Texture> {
    // Create raw device buffer.
    optick::event!("Core::create_texture");
    let raw = self.device.create_texture(&wgpu::TextureDescriptor {
      label,
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
    self.resources.create_proxy(raw)
  }

  /// Creates a new texture view.
  pub fn create_texture_view(&self, texture: ResourceRc<Texture>) -> ResourceRc<TextureView> {
    // Create raw device buffer.
    optick::event!("Core::create_texture_view");
    let raw = texture
      .get_raw()
      .create_view(&wgpu::TextureViewDescriptor::default());

    // Create proxy
    self.resources.create_proxy(raw)
  }

  /// Creates a new bind group layout.
  pub fn create_bind_group_layout(
    &self,
    desc: BindGroupLayoutDescriptor,
  ) -> ResourceRc<BindGroupLayout> {
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

    let raw = self
      .device
      .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &entries,
      });

    self.resources.create_proxy(raw)
  }

  /// Creates a new pipeline layout.
  pub fn create_pipeline_layout(
    &self,
    desc: PipelineLayoutDescriptor,
  ) -> ResourceRc<PipelineLayout> {
    optick::event!("Core::create_pipeline_layout");

    let raw = {
      let bindings = desc
        .bindings
        .iter()
        .map(|binding| binding.get_raw())
        .collect::<Vec<_>>();

      self
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
          label: None,
          bind_group_layouts: &bindings
            .iter()
            .map(|binding| &**binding)
            .collect::<Vec<_>>(),
          push_constant_ranges: &[],
        })
    };

    self.resources.create_proxy(raw)
  }

  /// Creates a new bind group.
  pub fn create_bind_group(&self, desc: BindGroupDescriptor) -> ResourceRc<BindGroup> {
    optick::event!("Core::create_bind_group");

    let raw = {
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

      self.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &*desc.layout.get_raw(),
        entries: &wgpu_entries
          .into_iter()
          .map(|(binding, resource)| wgpu::BindGroupEntry { binding, resource })
          .collect::<Vec<_>>(),
      })
    };

    self.resources.create_proxy(raw)
  }

  /// Creates a new bind group.
  pub fn create_render_pipeline(
    &self,
    desc: RenderPipelineDescriptor,
  ) -> ResourceRc<RenderPipeline> {
    optick::event!("Core::create_render_pipeline");

    let raw = {
      let vs = desc.vertex_shader.get_raw();
      let fs = desc.fragment_shader.get_raw();

      self
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
        })
    };

    self.resources.create_proxy(raw)
  }

  /// Creates a raw shader from vulkan compatible glsl.
  pub fn create_shader_from_glsl(
    &self,
    source: &str,
    name: &str,
    kind: ShaderKind,
  ) -> Result<ResourceRc<Shader>, ShaderError> {
    optick::event!("Core::create_shader");
    let mut compiler = Compiler::new().unwrap();

    // Compile to spir-v
    let spirv = compiler
      .compile_into_spirv(source, kind, name, "main", None)
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
      self
        .device
        .create_shader_module(&wgpu::ShaderModuleDescriptor {
          label: None,
          source: wgpu::util::make_spirv(&spirv),
          flags: wgpu::ShaderFlags::VALIDATION,
        })
    };

    // Create proxy
    Ok(self.resources.create_proxy(module))
  }
}

#[derive(Error, Debug)]
pub enum ShaderError {
  #[error("Failed to compile glsl shader to spir-v: {0}")]
  SpirVCompilationFailed(String),
}

pub enum TaskKind {
  Background,
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
  fn get_bind_group_lazy() -> ResourceRc<BindGroupLayout>;
}
