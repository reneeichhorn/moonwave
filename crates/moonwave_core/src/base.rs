use futures::Future;
use std::{
  sync::{Arc, RwLock},
  time::Instant,
};

use shaderc::Compiler;
pub use shaderc::ShaderKind;
use thiserror::Error;
use wgpu::{
  BufferDescriptor, Device, Queue, Surface, SwapChain, SwapChainDescriptor, SwapChainError,
};

pub use crate::resources::{Buffer, BufferUsage, ResourceRc, Shader, VertexAttribute};
use crate::{
  execution::Execution, resources::ResourceStorage, warn, EstimatedExecutionTime, Extension,
  ExtensionHost, World,
};

pub struct Core {
  device: Device,
  queue: Queue,
  swap_chain: SwapChain,
  sc_desc: SwapChainDescriptor,
  surface: Surface,
  execution: Execution,
  resources: ResourceStorage,
  extension_host: RwLock<ExtensionHost>,
  world: Arc<World>,
  last_frame: Instant,
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
      resources: ResourceStorage::new(),
      extension_host: RwLock::new(ExtensionHost::new()),
      world: Arc::new(World::new()),
    }
  }

  pub(crate) fn recreate_swap_chain(&mut self, width: u32, height: u32) {
    self.sc_desc.width = width;
    self.sc_desc.height = height;
    self.swap_chain = self.device.create_swap_chain(&self.surface, &self.sc_desc);
  }

  pub(crate) fn frame(&mut self, arced: Arc<Core>) -> Result<(), SwapChainError> {
    // Timing
    let time = Instant::now();
    let duration = time - self.last_frame;
    self.last_frame = time;

    // Next frame.
    let view = &self.swap_chain.get_current_frame()?.output.view;

    // Execute extensions
    {
      let ext_host = self.extension_host.read().unwrap();
      ext_host.before_tick(arced.clone());
    }

    // Execute ecs
    self.world.execute(arced, duration.as_millis() as u64);
    self.execution.block_ecs();
    unsafe {
      // Unsafe note: All execution in the ecs is halted at this point and is therefore safe to mutate.
      Arc::get_mut_unchecked(&mut self.world).handle_mutations();
    }

    self.execution.block_graph();
    self.execution.block_main();

    // Create new command encoder
    let mut encoder = self
      .device
      .create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Render Encoder"),
      });

    // Create render pass
    {
      let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Main Pass"),
        color_attachments: &[wgpu::RenderPassColorAttachmentDescriptor {
          attachment: view,
          resolve_target: None,
          ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color {
              r: 0.1,
              g: 0.2,
              b: 0.3,
              a: 1.0,
            }),
            store: true,
          },
        }],
        depth_stencil_attachment: None,
      });
    }

    // Submit command encoder
    self.queue.submit(std::iter::once(encoder.finish()));

    Ok(())
  }

  /// Registers a core extension
  pub fn add_extension<T: Extension>(&self, extension: T) {
    let mut host = self.extension_host.write().unwrap();
    host.add(extension);
  }

  /// Returns the ecs systems world container.
  pub fn get_world(&self) -> &Arc<World> {
    &self.world
  }

  /// Creates a new memory buffer on the GPU.
  pub async fn create_buffer(
    self: Arc<Self>,
    size: u64,
    mapped_at_creation: bool,
    usage: BufferUsage,
    label: Option<&str>,
  ) -> ResourceRc<Buffer> {
    // Prepare
    let label = label.map(|l| l.to_string());

    // Create raw device buffer.
    let self_cloned = self.clone();
    self
      .schedule_weighted_task(
        TaskKind::Background,
        async move {
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
    self: Arc<Self>,
    source: &str,
    name: &str,
    kind: ShaderKind,
  ) -> Result<ResourceRc<Shader>, ShaderError> {
    let source_string = source.to_string();
    let name_string = name.to_string();
    let label_string = name.to_string();
    let self_cloned = self.clone();

    self
      .schedule_weighted_task(
        TaskKind::Background,
        async move {
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
  ) -> impl Future<Output = F::Output>
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
  ) -> std::pin::Pin<Box<dyn Future<Output = F::Output>>>
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
