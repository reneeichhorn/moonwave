use crate::{base::Core, logger::init, ActorRc, Spawnable, TypedServiceIntoHost};
use legion::{systems::CommandBuffer, Resources};
use log::debug;
use wgpu::SwapChainError;
use winit::{
  dpi::PhysicalSize,
  event::*,
  event_loop::{ControlFlow, EventLoop},
  window::{Window, WindowBuilder},
};

pub struct Application {
  event_loop: Option<EventLoop<()>>,
  window: Window,
  win_size: PhysicalSize<u32>,
}

impl Application {
  pub fn new() -> Self {
    // Initialize core logging systems.
    init();

    // Create window
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new().build(&event_loop).unwrap();
    let win_size = window.inner_size();

    // Block main thread for initial base wgpu creation.
    let (surface, device, queue, swap_chain, sc_desc) = futures::executor::block_on(async {
      // Handle wgpu initialization
      let instance = wgpu::Instance::new(wgpu::BackendBit::all());
      let surface = unsafe { instance.create_surface(&window) };
      let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
          power_preference: wgpu::PowerPreference::default(),
          compatible_surface: Some(&surface),
        })
        .await
        .unwrap();
      let (device, queue) = adapter
        .request_device(
          &wgpu::DeviceDescriptor {
            label: Some("Render Device"),
            features: wgpu::Features::empty(),
            limits: wgpu::Limits::default(),
          },
          None, // Trace path
        )
        .await
        .unwrap();

      // Logging
      debug!(
        "Device created:\n\tFeatures: {:?}\n\tLimits: {:?}",
        device.features(),
        device.limits()
      );

      // Create swap chain
      let sc_format = adapter.get_swap_chain_preferred_format(&surface);
      let sc_desc = wgpu::SwapChainDescriptor {
        usage: wgpu::TextureUsage::RENDER_ATTACHMENT,
        format: sc_format,
        width: win_size.width,
        height: win_size.height,
        present_mode: wgpu::PresentMode::Mailbox,
      };
      let swap_chain = device.create_swap_chain(&surface, &sc_desc);

      (surface, device, queue, swap_chain, sc_desc)
    });

    Core::initialize(device, queue, swap_chain, sc_desc, surface);

    Self {
      event_loop: Some(event_loop),
      window,
      win_size,
    }
  }

  pub fn set_title(&mut self, title: &str) {
    self.window.set_title(title);
  }

  pub fn register_service<T: TypedServiceIntoHost>(&self, system: T) {
    Core::get_instance().get_service_locator().register(system);
  }

  fn handle_update_size(&mut self) {
    self.win_size = self.window.inner_size();

    // This is safe to the way the threading model is built. This will be always executed on the main thread
    // Swapchain recreation is also garantued to be not touched during any background tasks.
    Core::get_instance_mut_unstable()
      .recreate_swap_chain(self.win_size.width, self.win_size.height);
  }

  fn render(&mut self) -> Result<(), SwapChainError> {
    // Optick
    optick::next_frame();

    // Execute core application.
    // This is safe to the way the threading model is built. This will be always executed on the main thread
    // Only the frame is being accessed which is never touched during any background tasks.
    Core::get_instance_mut_unstable().frame()
  }

  /// Spawns a new actor into the application.
  /// Panics if called after application started.
  pub fn add_actor<T: Spawnable>(&self, actor: T) -> ActorRc<T> {
    let mut cmd = CommandBuffer::new(&Core::get_instance().get_world().world);
    let rc = actor.spawn(None, 0, &mut cmd);
    cmd.flush(
      &mut Core::get_instance_mut_unstable().get_world_mut().world,
      &mut Resources::default(),
    );
    rc
  }

  /// Starts execution of the application, will block current thread until application exits.
  pub fn run(mut self) {
    // Build event loop.
    let event_loop = self.event_loop.take().unwrap();

    // Run window
    event_loop.run(move |event, _, control_flow| match event {
      Event::WindowEvent {
        ref event,
        window_id,
      } if window_id == self.window.id() => match event {
        WindowEvent::Resized(_physical_size) => {
          self.handle_update_size();
        }
        WindowEvent::ScaleFactorChanged { .. } => {
          self.handle_update_size();
        }
        WindowEvent::CloseRequested => *control_flow = ControlFlow::Exit,
        /*
        WindowEvent::KeyboardInput { input, .. } => match input {
          _ => {}
        },
        */
        _ => {}
      },
      Event::RedrawRequested(_) => match self.render() {
        Ok(_) => {}
        Err(SwapChainError::Lost) => self.handle_update_size(),
        Err(wgpu::SwapChainError::OutOfMemory) => *control_flow = ControlFlow::Exit,
        Err(e) => eprintln!("{:?}", e),
      },
      Event::MainEventsCleared => {
        self.window.request_redraw();
      }
      _ => {}
    });
  }
}
