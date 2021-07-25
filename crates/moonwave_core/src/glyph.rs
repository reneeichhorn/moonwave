use std::sync::Arc;

use moonwave_common::*;
use moonwave_render::{execute_wgpu_async, CommandEncoderOutput, FrameGraphNode, FrameNodeValue};
use parking_lot::RwLock;
use wgpu::{LoadOp, Operations, RenderPassDescriptor};
use wgpu_glyph::{GlyphBrush, GlyphBrushBuilder, Section, Text};

use crate::Core;

use std::cell::RefCell;
thread_local! {
  static FONT_RENDERING_STAGING_BELL: RefCell<wgpu::util::StagingBelt> = RefCell::new(wgpu::util::StagingBelt::new(1024));
}

const FONT_SCENE_SCALE: f32 = 200.0;

pub struct Glyph {
  brush: Arc<RwLock<GlyphBrush<()>>>,
  brush2d: Arc<RwLock<GlyphBrush<()>>>,
}

impl Glyph {
  pub fn new(font: &'static [u8]) -> Self {
    let core = Core::get_instance();

    let font = ab_glyph::FontArc::try_from_slice(font).unwrap();

    let brush = GlyphBrushBuilder::using_font(font.clone())
      .build(&core.device, wgpu::TextureFormat::Bgra8UnormSrgb);
    let brush2d =
      GlyphBrushBuilder::using_font(font).build(&core.device, wgpu::TextureFormat::Bgra8UnormSrgb);

    Self {
      brush: Arc::new(RwLock::new(brush)),
      brush2d: Arc::new(RwLock::new(brush2d)),
    }
  }

  pub fn queue_scene_text(
    &self,
    text: &str,
    position: Vector3<f32>,
    color: Vector4<f32>,
    size: f32,
  ) {
    let scaled_position = position.xy() * FONT_SCENE_SCALE;
    let section = Section {
      screen_position: scaled_position.into(),
      text: vec![Text::new(text)
        .with_color([color.x, color.y, color.z, color.w])
        .with_scale(size * FONT_SCENE_SCALE)
        .with_z(position.z * FONT_SCENE_SCALE)],
      ..Section::default()
    };
    let mut brush = self.brush.write();
    brush.queue(section);
  }

  pub fn queue_2d_text(&self, text: &str, position: Vector2<f32>, color: Vector4<f32>, size: f32) {
    let section = Section {
      screen_position: position.into(),
      text: vec![Text::new(text)
        .with_color([color.x, color.y, color.z, color.w])
        .with_scale(size * FONT_SCENE_SCALE)],
      ..Section::default()
    };
    let mut brush = self.brush2d.write();
    brush.queue(section);
  }

  pub fn create_frame_node(&self, view: Matrix4<f32>, projection: Matrix4<f32>) -> GlyphFrameNode {
    let scale = Matrix4::from_scale(1.0 / FONT_SCENE_SCALE);
    let rotation = Matrix4::from_angle_x(Deg(180.0));
    /*
    let billboard = Matrix4::new(
      -1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, view[3][0], view[3][1],
      view[3][2], view[3][3],
    );
    */
    GlyphFrameNode {
      brush: self.brush.clone(),
      brush2d: self.brush2d.clone(),
      transform: projection * view * scale * rotation,
    }
  }
}

pub struct GlyphFrameNode {
  brush: Arc<RwLock<GlyphBrush<()>>>,
  brush2d: Arc<RwLock<GlyphBrush<()>>>,
  transform: Matrix4<f32>,
}

impl GlyphFrameNode {
  pub const INPUT_TEXTURE: usize = 0;
  pub const OUTPUT_TEXTURE: usize = 0;
}

impl FrameGraphNode for GlyphFrameNode {
  fn execute_raw(
    &self,
    inputs: &[Option<FrameNodeValue>],
    outputs: &mut [Option<FrameNodeValue>],
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    _sc_frame: &wgpu::SwapChainFrame,
  ) -> CommandEncoderOutput {
    let texture_in = inputs[Self::INPUT_TEXTURE]
      .as_ref()
      .unwrap()
      .get_sampled_texture();

    let core = Core::get_instance();

    let mut encoder = core
      .device
      .create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("CommandEncoderGlyph"),
      });

    {
      let _rp = encoder.begin_render_pass(&RenderPassDescriptor {
        color_attachments: &[wgpu::RenderPassColorAttachment {
          view: &*texture_in.view.get_raw(),
          resolve_target: None,
          ops: Operations {
            load: LoadOp::Clear(wgpu::Color::TRANSPARENT),
            store: true,
          },
        }],
        ..Default::default()
      });
    }

    let mut brush = self.brush.write();
    let mut brush2d = self.brush2d.write();
    FONT_RENDERING_STAGING_BELL.with(|belt_bridge| {
      let mut belt = belt_bridge.borrow_mut();
      // Render 3d scene texts
      let transform = self.transform;
      brush
        .draw_queued_with_transform(
          device,
          &mut *belt,
          &mut encoder,
          texture_in.view.get_raw(),
          [
            transform[0][0],
            transform[0][1],
            transform[0][2],
            transform[0][3],
            transform[1][0],
            transform[1][1],
            transform[1][2],
            transform[1][3],
            transform[2][0],
            transform[2][1],
            transform[2][2],
            transform[2][3],
            transform[3][0],
            transform[3][1],
            transform[3][2],
            transform[3][3],
          ],
        )
        .expect("Render font quads");

      // Render 2d texts
      let target_size = core.get_swap_chain_size();
      brush2d
        .draw_queued(
          device,
          &mut *belt,
          &mut encoder,
          texture_in.view.get_raw(),
          target_size.x,
          target_size.y,
        )
        .expect("Render font quads");

      // Finish and execute belt and other commands.
      belt.finish();
      queue.submit(vec![encoder.finish()]);

      execute_wgpu_async(device, belt.recall());
    });

    outputs[Self::OUTPUT_TEXTURE] = inputs[Self::INPUT_TEXTURE].clone();

    CommandEncoderOutput::empty()
  }
}
