use futures::{executor::block_on, Future};
use moonwave_common::Vector2;
use moonwave_render::{DeviceHost, FrameGraph};
use std::{
  marker::PhantomData,
  sync::{Arc, RwLock},
  time::Instant,
};

use shaderc::Compiler;
pub use shaderc::ShaderKind;
use thiserror::Error;
use wgpu::{
  BufferDescriptor, Device, Queue, Surface, SwapChain, SwapChainDescriptor, SwapChainError,
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
