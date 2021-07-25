use std::sync::Arc;

use generational_arena::Arena;
use lazy_static::lazy_static;
use legion::{world::SubWorld, *};
use moonwave_common::*;
use moonwave_core::{
  Core, Extension, Glyph, GlyphFrameNode, PresentToScreen, SystemFactory, TextureGeneratorHost,
  TextureGeneratorNode, WrappedSystem,
};
use parking_lot::RwLock;

use crate::{Camera, MainCameraTag};

pub struct ImmediateModeDebugger {
  glyph: Glyph,
  arena: RwLock<Arena<DebuggerObject>>,
}

impl ImmediateModeDebugger {
  fn new() -> Self {
    let font = include_bytes!("./FiraMono-Medium.ttf");
    let glyph = Glyph::new(font);

    Self {
      glyph,
      arena: RwLock::new(Arena::new()),
    }
  }

  pub fn draw_forever(&self, obj: DebuggerObject) {
    let mut arena = self.arena.write();
    arena.insert(obj);
  }

  pub fn create_extension(&self) -> ImmediateModeDebuggerExt {
    ImmediateModeDebuggerExt { host: None }
  }
}

pub enum DebuggerObject {
  UIText {
    color: Vector4<f32>,
    size: f32,
    text: String,
  },
  SceneText {
    position: Vector3<f32>,
    color: Vector4<f32>,
    size: f32,
    text: String,
  },
}

lazy_static! {
  pub static ref IMMEDIATE_MODE_DEBUGGER: ImmediateModeDebugger = ImmediateModeDebugger::new();
}

pub struct ImmediateModeDebuggerExt {
  host: Option<Arc<TextureGeneratorHost>>,
}

impl Extension for ImmediateModeDebuggerExt {
  fn init(&mut self) {
    // Create host texture
    let host = TextureGeneratorHost::new(
      moonwave_core::TextureSize::FullScreen,
      moonwave_resources::TextureFormat::Bgra8UnormSrgb,
    );
    self.host = Some(host);

    // Add system
    Core::get_instance().get_world().add_system_to_stage(
      IMDTickSystem {
        host: self.host.clone().unwrap(),
      },
      moonwave_core::SystemStage::Rendering,
    )
  }
}

#[system]
#[read_component(MainCameraTag)]
#[read_component(Camera)]
fn immediate_mode_debugger_tick(world: &mut SubWorld, #[state] host: &Arc<TextureGeneratorHost>) {
  let core = Core::get_instance();
  //let win_size = core.get_swap_chain_size();
  let frame_graph = core.get_frame_graph();

  // Get main camera.
  let mut main_cam_query = <(&Camera, &MainCameraTag)>::query();
  let main_cam = main_cam_query.iter(world).next();
  if main_cam.is_none() {
    return;
  }
  let camera_view = main_cam.unwrap().0.uniform.get().view;
  let camera_proj = main_cam.unwrap().0.uniform.get().projection;

  // Prepare debug objects
  let objects = IMMEDIATE_MODE_DEBUGGER.arena.read();
  let mut text_stack_height = 0.0;
  for (_, object) in objects.iter() {
    match object {
      DebuggerObject::SceneText {
        position,
        color,
        size,
        text,
      } => {
        IMMEDIATE_MODE_DEBUGGER
          .glyph
          .queue_scene_text(text.as_str(), *position, *color, *size);
      }
      DebuggerObject::UIText { color, size, text } => {
        IMMEDIATE_MODE_DEBUGGER.glyph.queue_2d_text(
          text.as_str(),
          Vector2::new(0.0, text_stack_height),
          *color,
          *size,
        );
        text_stack_height += *size;
      }
      _ => panic!("Unknown debug object"),
    }
  }

  // Build texture node.
  let input_texture = host.create_node();
  let input_texture_index = frame_graph.add_node(input_texture, "IMDTextureHost");

  // Build graph for text rendering
  let node = IMMEDIATE_MODE_DEBUGGER
    .glyph
    .create_frame_node(camera_view, camera_proj);
  let node_index = frame_graph.add_node(node, "IMDGlyph");
  frame_graph
    .connect(
      input_texture_index,
      TextureGeneratorNode::OUTPUT_TEXTURE,
      node_index,
      GlyphFrameNode::INPUT_TEXTURE,
    )
    .unwrap();

  // Connect to to screen output.
  frame_graph
    .connect(
      node_index,
      GlyphFrameNode::OUTPUT_TEXTURE,
      frame_graph.get_end_node(),
      PresentToScreen::INPUT_TEXTURE_UI + 1,
    )
    .unwrap();
}

struct IMDTickSystem {
  host: Arc<TextureGeneratorHost>,
}
impl SystemFactory for IMDTickSystem {
  fn create_system(&self) -> WrappedSystem {
    WrappedSystem(Box::new(immediate_mode_debugger_tick_system(
      self.host.clone(),
    )))
  }
}
