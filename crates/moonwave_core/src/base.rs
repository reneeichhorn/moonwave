use itertools::Itertools;
use lazy_static::__Deref;
use moonwave_common::Vector2;
use moonwave_render::{CommandEncoder, DeviceHost, FrameGraph};
use parking_lot::Mutex;
use std::{
  collections::HashMap,
  num::NonZeroU32,
  sync::{
    atomic::{AtomicU64, Ordering},
    Arc, RwLock,
  },
  time::Instant,
};
use wgpu_mipmap::{MipmapGenerator, RecommendedMipmapGenerator};

use shaderc::Compiler;
pub use shaderc::ShaderKind;
use thiserror::Error;
use wgpu::{
  util::DeviceExt, BufferDescriptor, CommandBuffer, Device, Queue, Surface, SwapChain,
  SwapChainDescriptor, SwapChainError, TextureViewDimension,
};

use crate::{
  execution::Execution, warn, Extension, ExtensionHost, PresentToScreen, ServiceLocator, World,
};

use moonwave_resources::*;

static mut CORE: Option<Core> = None;

pub struct Core {
  pub(crate) device: Device,
  queue: Queue,
  swap_chain: SwapChain,
  sc_desc: SwapChainDescriptor,
  surface: Surface,
  resources: ResourceStorage,
  extension_host: RwLock<ExtensionHost>,
  mip_generator: RecommendedMipmapGenerator,
  elapsed_time: u64,
  graph: Option<FrameGraph>,
  world: World,
  last_frame: Instant,
  service_locator: ServiceLocator,
  execution: Execution,
  gp_resources: Option<GPResources>,
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
      mip_generator: RecommendedMipmapGenerator::new(&device),
      last_frame: Instant::now(),
      elapsed_time: 0,
      swap_chain,
      sc_desc,
      device,
      queue,
      surface,
      graph: None,
      gp_resources: None,
      resources: ResourceStorage::new(),
      extension_host: RwLock::new(ExtensionHost::new()),
      service_locator: ServiceLocator::new(),
      execution: Execution::new(8),
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
    // Build static core and create new framegraph.
    unsafe {
      CORE = Some(Core::new(device, queue, swap_chain, sc_desc, surface));
    }

    let core = Self::get_instance();

    // Build general purpose texture sampler.
    let bind_group_layout_desc = BindGroupLayoutDescriptor::new()
      .add_entry(0, BindGroupLayoutEntryType::SingleTexture)
      .add_entry(1, BindGroupLayoutEntryType::Sampler);
    let sampled_texture_bind_group_layout = core.create_bind_group_layout(bind_group_layout_desc);

    // Store mutably
    unsafe {
      CORE.as_mut().unwrap().gp_resources = Some(GPResources {
        sampled_texture_bind_group_layout,
        sampled_texture_array_bind_group_layout: Mutex::new(HashMap::new()),
      });
      CORE.as_mut().unwrap().graph = Some(FrameGraph::new(PresentToScreen::new()));
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

  #[inline]
  pub fn get_gp_resources(&self) -> &GPResources {
    self.gp_resources.as_ref().unwrap()
  }

  #[inline]
  pub fn get_elapsed_time(&self) -> u64 {
    self.elapsed_time
  }

  pub(crate) fn recreate_swap_chain(&mut self, width: u32, height: u32) {
    self.sc_desc.width = width;
    self.sc_desc.height = height;
    self.swap_chain = self.device.create_swap_chain(&self.surface, &self.sc_desc);
  }

  pub fn get_swap_chain_size(&self) -> Vector2<u32> {
    Vector2::new(self.sc_desc.width, self.sc_desc.height)
  }

  pub(crate) fn before_run(&self) {
    optick::event!("Core::extensions::init");
    let mut ext_host = self.extension_host.write().unwrap();
    ext_host.init();
  }

  pub(crate) fn frame(&mut self) -> Result<(), SwapChainError> {
    // Timing
    let time = Instant::now();
    let duration = time - self.last_frame;
    self.last_frame = time;
    self.elapsed_time = duration.as_micros() as u64;

    // Next frame.
    let swap_frame = Arc::new(self.swap_chain.get_current_frame()?);

    // Execute extensions
    {
      optick::event!("Core::extensions::before_tick");
      let mut ext_host = self.extension_host.write().unwrap();
      ext_host.before_tick();
    }

    // Execute ecs
    {
      optick::event!("Core::frame::ecs_tick");
      self
        .world
        .tick(self.elapsed_time, self.execution.get_frame_thread_pool());
    }

    // Execute graph
    {
      optick::event!("Core::frame::execute_graph");
      self.graph.as_mut().unwrap().execute(
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

    CURRENT_FRAME.fetch_add(1, Ordering::Relaxed);

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
    &self.graph.as_ref().unwrap()
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
        depth_or_array_layers: 1,
      },
      usage,
      format,
    });

    // Create proxy
    self.resources.create_proxy(raw)
  }

  pub fn exec_with_encoder<'a, F: FnOnce(&mut CommandEncoder<'a>)>(&'a self, f: F) {
    let mut encoder = CommandEncoder::new(&self.device, "withEncoderFunction");
    f(&mut encoder);
    let out = encoder.finish();
    self.queue.submit(out.command_buffer);
  }

  pub fn upload_texture(
    &self,
    texture: ResourceRc<Texture>,
    usage: TextureUsage,
    format: TextureFormat,
    size: Vector2<u32>,
    buffer: &[u8],
    bytes_per_row: usize,
  ) {
    // Fill texture
    self.queue.write_texture(
      wgpu::ImageCopyTexture {
        texture: &*texture.get_raw(),
        mip_level: 0,
        origin: wgpu::Origin3d::ZERO,
      },
      buffer,
      wgpu::ImageDataLayout {
        bytes_per_row: NonZeroU32::new(bytes_per_row as u32),
        offset: 0,
        rows_per_image: NonZeroU32::new(size.y),
      },
      wgpu::Extent3d {
        width: size.x,
        height: size.y,
        depth_or_array_layers: 1,
      },
    );

    // Calculate mips
    let highest_size = size.x.max(size.y);
    let mips = (highest_size as f32).log2().floor() as u32;

    // Generate mips and submit write.
    let desc = wgpu::TextureDescriptor {
      label: None,
      mip_level_count: mips + 1,
      sample_count: 1,
      dimension: wgpu::TextureDimension::D2,
      size: wgpu::Extent3d {
        width: size.x,
        height: size.y,
        depth_or_array_layers: 1,
      },
      usage: wgpu::TextureUsage::COPY_DST | wgpu::TextureUsage::RENDER_ATTACHMENT | usage,
      format,
    };
    let mut encoder = self.device.create_command_encoder(&Default::default());
    self
      .mip_generator
      .generate(&self.device, &mut encoder, &*texture.get_raw(), &desc)
      .unwrap();
    self.queue.submit(std::iter::once(encoder.finish()));
  }

  pub fn create_inited_sampled_texture(
    &self,
    label: Option<&str>,
    usage: TextureUsage,
    format: TextureFormat,
    size: Vector2<u32>,
    buffer: &[u8],
    bytes_per_row: usize,
  ) -> SampledTexture {
    optick::event!("Core::create_inited_texture");

    // Calculate mips
    let highest_size = size.x.max(size.y);
    let mips = (highest_size as f32).log2().floor() as u32;

    // Create empty texture.
    let desc = wgpu::TextureDescriptor {
      label,
      mip_level_count: mips + 1,
      sample_count: 1,
      dimension: wgpu::TextureDimension::D2,
      size: wgpu::Extent3d {
        width: size.x,
        height: size.y,
        depth_or_array_layers: 1,
      },
      usage: wgpu::TextureUsage::COPY_DST | wgpu::TextureUsage::RENDER_ATTACHMENT | usage,
      format,
    };
    let raw = self.device.create_texture(&desc);

    // Fill texture
    self.queue.write_texture(
      wgpu::ImageCopyTexture {
        texture: &raw,
        mip_level: 0,
        origin: wgpu::Origin3d::ZERO,
      },
      buffer,
      wgpu::ImageDataLayout {
        bytes_per_row: NonZeroU32::new(bytes_per_row as u32),
        offset: 0,
        rows_per_image: NonZeroU32::new(size.y),
      },
      wgpu::Extent3d {
        width: size.x,
        height: size.y,
        depth_or_array_layers: 1,
      },
    );

    // Generate mips and submit write.
    let mut encoder = self.device.create_command_encoder(&Default::default());
    self
      .mip_generator
      .generate(&self.device, &mut encoder, &raw, &desc)
      .unwrap();
    self.queue.submit(std::iter::once(encoder.finish()));

    // Create proxy
    let texture = self.resources.create_proxy(raw);

    // Create sampling
    let gp_resources = self.get_gp_resources();
    let view = self.create_texture_view(texture.clone());
    let sampler = self.create_sampler();
    let bind_group = self.create_bind_group(
      BindGroupDescriptor::new(gp_resources.sampled_texture_bind_group_layout.clone())
        .add_texture_binding(0, view.clone())
        .add_sampler_binding(1, sampler.clone()),
    );

    SampledTexture {
      view,
      texture,
      sampler,
      bind_group,
    }
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

  /// Creates a new texture sampler.
  pub fn create_sampler(&self) -> ResourceRc<Sampler> {
    let raw = self.device.create_sampler(&wgpu::SamplerDescriptor {
      address_mode_u: wgpu::AddressMode::Repeat,
      address_mode_v: wgpu::AddressMode::Repeat,
      ..Default::default()
    });
    self.resources.create_proxy(raw)
  }

  pub fn create_sampled_texture(
    &self,
    label: Option<&str>,
    usage: TextureUsage,
    format: TextureFormat,
    size: Vector2<u32>,
    mips: u32,
  ) -> SampledTexture {
    let gp_resources = self.get_gp_resources();

    let texture = self.create_texture(label, usage, format, size, mips);
    let view = self.create_texture_view(texture.clone());
    let sampler = self.create_sampler();
    let bind_group = self.create_bind_group(
      BindGroupDescriptor::new(gp_resources.sampled_texture_bind_group_layout.clone())
        .add_texture_binding(0, view.clone())
        .add_sampler_binding(1, sampler.clone()),
    );

    SampledTexture {
      view,
      texture,
      sampler,
      bind_group,
    }
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
        count: match entry.ty {
          BindGroupLayoutEntryType::ArrayTexture(size) => {
            Some(NonZeroU32::new(size as u32).unwrap())
          }
          _ => None,
        },
        visibility: wgpu::ShaderStage::all(),
        ty: match entry.ty {
          BindGroupLayoutEntryType::UniformBuffer => wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
          },
          BindGroupLayoutEntryType::Sampler => wgpu::BindingType::Sampler {
            comparison: false,
            filtering: true,
          },
          BindGroupLayoutEntryType::SingleTexture => wgpu::BindingType::Texture {
            multisampled: false,
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
          },
          BindGroupLayoutEntryType::ArrayTexture(_) => wgpu::BindingType::Texture {
            multisampled: false,
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2Array,
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
      let views_raw = Vec::new();

      // Validate to ensure unsafe block below is valid
      std::assert!(
        desc
          .entries
          .iter()
          .filter(|(_, entry)| matches!(entry, BindGroupEntry::TextureArray(_)))
          .collect_vec()
          .len()
          <= 1
      );

      // Create bind group entry
      let mut wgpu_entries = Vec::with_capacity(desc.entries.len());
      for (binding, entry) in &desc.entries {
        let raw_entry = match entry {
          BindGroupEntry::Buffer(buffer) => wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: buffer.get_raw(),
            offset: 0,
            size: None,
          }),
          BindGroupEntry::Texture(texture) => wgpu::BindingResource::TextureView(texture.get_raw()),
          BindGroupEntry::TextureArray(textures) => {
            unsafe {
              let ptr = &views_raw as *const Vec<_>;
              let views_raw = ptr as *mut Vec<_>;
              (*views_raw).extend(textures.iter().map(|t| t.get_raw()));
            }
            wgpu::BindingResource::TextureViewArray(&views_raw)
          }
          BindGroupEntry::Sampler(sampler) => wgpu::BindingResource::Sampler(sampler.get_raw()),
        };
        wgpu_entries.push((*binding, raw_entry));
      }

      // Bind group device
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

      let attributes = desc.vertex_desc.clone().map(|vertex_desc| {
        vertex_desc
          .attributes
          .iter()
          .map(|attr| wgpu::VertexAttribute {
            shader_location: attr.location as u32,
            offset: attr.offset,
            format: attr.format.to_wgpu(),
          })
          .collect::<Vec<_>>()
      });

      let vs_buffer = desc
        .vertex_desc
        .map(|vertex_desc| wgpu::VertexBufferLayout {
          array_stride: vertex_desc.stride,
          step_mode: wgpu::InputStepMode::Vertex,
          attributes: attributes.as_ref().unwrap(),
        });

      let mut buffers = Vec::with_capacity(1);
      if let Some(vs) = vs_buffer {
        buffers.push(vs);
      }

      self
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
          label: None,
          layout: Some(&*desc.layout.get_raw()),
          multisample: wgpu::MultisampleState::default(),
          vertex: wgpu::VertexState {
            module: &*vs,
            entry_point: "main",
            buffers: &buffers,
          },
          primitive: wgpu::PrimitiveState {
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Back),
            polygon_mode: wgpu::PolygonMode::Fill,
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            clamp_depth: false,
            conservative: false,
          },
          depth_stencil: desc.depth.map(|depth| wgpu::DepthStencilState {
            bias: wgpu::DepthBiasState::default(),
            stencil: wgpu::StencilState::default(),
            format: depth,
            depth_compare: wgpu::CompareFunction::Less,
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
                blend: Some(wgpu::BlendState {
                  color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                  },
                  alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                  },
                }),
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

    //println!("=============\n{}\n=========\n", source);
    // Compile to spir-v
    let spirv = compiler
      .compile_into_spirv(source, kind, name, "main", None)
      .map_err(|err| ShaderError::SpirVCompilationFailed(err.to_string(), source.to_string()))?;

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
          flags: wgpu::ShaderFlags::empty(),
        })
    };

    // Create proxy
    Ok(self.resources.create_proxy(module))
  }
}

pub struct GPResources {
  /// A bind group used for simple single texture binding.
  pub sampled_texture_bind_group_layout: ResourceRc<BindGroupLayout>,
  /// A bind group used for array texture binding
  pub sampled_texture_array_bind_group_layout: Mutex<HashMap<usize, ResourceRc<BindGroupLayout>>>,
}

impl GPResources {
  pub fn get_sampled_texture_array_bind_group_layout(
    &self,
    size: usize,
  ) -> ResourceRc<BindGroupLayout> {
    let mut cache = self.sampled_texture_array_bind_group_layout.lock();
    if let Some(layout) = cache.get(&size) {
      return layout.clone();
    }

    let bind_group_layout_desc = BindGroupLayoutDescriptor::new()
      .add_entry(0, BindGroupLayoutEntryType::ArrayTexture(size))
      .add_entry(1, BindGroupLayoutEntryType::Sampler);
    let bind_group_layout = Core::get_instance().create_bind_group_layout(bind_group_layout_desc);
    cache.insert(size, bind_group_layout.clone());
    bind_group_layout
  }
}

#[derive(Error, Debug)]
pub enum ShaderError {
  #[error("Failed to compile glsl shader to spir-v: {0}\n\nCode: {1}")]
  SpirVCompilationFailed(String, String),
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

static CURRENT_FRAME: AtomicU64 = AtomicU64::new(0);

pub struct OnceInFrame {
  last_execution: AtomicU64,
}
impl OnceInFrame {
  pub fn new() -> Self {
    Self {
      last_execution: AtomicU64::new(0),
    }
  }

  pub fn once<F: Fn()>(&self, f: F) {
    let last_execution = self.last_execution.load(Ordering::Relaxed);
    let current = CURRENT_FRAME.load(Ordering::Relaxed);
    if current > last_execution {
      self.last_execution.store(last_execution, Ordering::Relaxed);
      f();
    }
  }
}
