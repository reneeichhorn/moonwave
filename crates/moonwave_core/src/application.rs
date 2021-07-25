use crate::{base::Core, logger::init, ActorRc, Extension, Spawnable, TypedServiceIntoHost};
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
  #[cfg(feature = "renderdochost")]
  renderdoc: renderdoc::RenderDoc<renderdoc::V110>,

  event_loop: Option<EventLoop<()>>,
  window: Window,
  win_size: PhysicalSize<u32>,
}

impl Application {
  pub fn new() -> Self {
    // Initialize core logging systems.
    init();

    // Render doc support
    #[cfg(feature = "renderdochost")]
    let renderdoc = renderdoc::RenderDoc::new().expect("Unable to connect");

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
            features: wgpu::Features::NON_FILL_POLYGON_MODE
              | wgpu::Features::SAMPLED_TEXTURE_BINDING_ARRAY,
            limits: wgpu::Limits {
              max_sampled_textures_per_shader_stage: 128,
              ..wgpu::Limits::default()
            },
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
        format: sc_format.unwrap(),
        width: win_size.width,
        height: win_size.height,
        present_mode: wgpu::PresentMode::Mailbox,
      };
      let swap_chain = device.create_swap_chain(&surface, &sc_desc);

      (surface, device, queue, swap_chain, sc_desc)
    });

    Core::initialize(device, queue, swap_chain, sc_desc, surface);

    Self {
      #[cfg(feature = "renderdochost")]
      renderdoc,
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
    Core::get_instance()
      .get_world()
      .add_command_buffer(cmd, false, None);
    rc
  }

  /// Registers a core extension
  pub fn add_extension<T: Extension>(&self, extension: T) {
    Core::get_instance().add_extension(extension);
  }

  /// Starts execution of the application, will block current thread until application exits.
  pub fn run(mut self) {
    // Execute extensions
    Core::get_instance().before_run();

    #[cfg(feature = "renderdochost")]
    {
      let mut rd = &mut self.renderdoc;
      rd.set_focus_toggle_keys(&[renderdoc::InputButton::F]);
      rd.set_capture_keys(&[renderdoc::InputButton::C]);
      rd.set_capture_option_u32(renderdoc::CaptureOption::AllowVSync, 1);
      rd.set_capture_option_u32(renderdoc::CaptureOption::ApiValidation, 1);
      rd.mask_overlay_bits(renderdoc::OverlayBits::ALL, renderdoc::OverlayBits::ALL);
    }

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
        WindowEvent::KeyboardInput { input, .. } => {
          #[cfg(feature = "renderdochost")]
          if input.virtual_keycode == Some(VirtualKeyCode::F10)
            && input.state == ElementState::Released
          {
            self.renderdoc.launch_replay_ui(true, None).unwrap();
          }

          let event = KeyboardEvent {
            key: input.virtual_keycode,
            state: input.state,
          };

          Core::get_instance().get_world().publish_event(event);
        }
        _ => {}
      },
      Event::DeviceEvent { event, .. } => {
        Core::get_instance().get_world().publish_event(event);
      }
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

pub use winit::event::{DeviceEvent, ElementState, VirtualKeyCode};

#[derive(Clone)]
pub struct KeyboardEvent {
  pub key: Option<VirtualKeyCode>,
  pub state: ElementState,
}
