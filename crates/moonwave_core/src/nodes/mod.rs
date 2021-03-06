use crate::Core;
use moonwave_common::Vector2;
use moonwave_render::{CommandEncoder, CommandEncoderOutput, FrameGraphNode, FrameNodeValue};
use moonwave_resources::{ResourceRc, Texture, TextureFormat, TextureUsage, TextureView};
use parking_lot::Mutex;
use std::sync::Arc;

pub struct PresentToScreen {}

impl PresentToScreen {
  pub const INPUT_TEXTURE: usize = 0;
}

impl FrameGraphNode for PresentToScreen {
  fn execute_raw(
    &self,
    _inputs: &[Option<FrameNodeValue>],
    _outputs: &mut [Option<FrameNodeValue>],
    device: &wgpu::Device,
    _queue: &wgpu::Queue,
    sc_frame: &wgpu::SwapChainFrame,
  ) -> CommandEncoderOutput {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
      label: Some("CommandEncoderPresentToScreen"),
    });

    {
      let _rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("RenderPassPresentToScreen"),
        color_attachments: &[wgpu::RenderPassColorAttachmentDescriptor {
          resolve_target: None,
          attachment: &sc_frame.output.view,
          ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
            store: true,
          },
        }],
        depth_stencil_attachment: None,
      });
    }

    CommandEncoderOutput::from_raw(encoder.finish())
  }
}

pub enum TextureSize {
  FullScreen,
  Custom(Vector2<u32>),
}

impl TextureSize {
  fn get_actual_size(&self) -> Vector2<u32> {
    match self {
      TextureSize::Custom(size) => *size,
      TextureSize::FullScreen => Core::get_instance().get_swap_chain_size(),
    }
  }
}

pub struct TextureGeneratorHost {
  size: TextureSize,
  format: TextureFormat,
  active: Mutex<(Vector2<u32>, ResourceRc<Texture>, ResourceRc<TextureView>)>,
}

impl TextureGeneratorHost {
  pub fn new(size: TextureSize, format: TextureFormat) -> Arc<Self> {
    let core = Core::get_instance();
    let actual_size = size.get_actual_size();
    let texture = core.create_texture(
      None,
      TextureUsage::RENDER_ATTACHMENT | TextureUsage::SAMPLED,
      format,
      actual_size,
      1,
    );
    let view = core.create_texture_view(texture.clone());

    Arc::new(Self {
      format,
      size,
      active: Mutex::new((actual_size, texture, view)),
    })
  }

  pub fn create_node(self: &Arc<Self>) -> TextureGeneratorNode {
    TextureGeneratorNode(self.clone())
  }
}

pub struct TextureGeneratorNode(Arc<TextureGeneratorHost>);

impl TextureGeneratorNode {
  pub const OUTPUT_TEXTURE: usize = 0;
}

impl FrameGraphNode for TextureGeneratorNode {
  fn execute(
    &self,
    _inputs: &[Option<FrameNodeValue>],
    outputs: &mut [Option<FrameNodeValue>],
    _encoder: &mut CommandEncoder,
  ) {
    // Recreate texture if resolution changed.
    let size = self.0.size.get_actual_size();
    let mut active = self.0.active.lock();
    if size != active.0 {
      let core = Core::get_instance();
      let texture = core.create_texture(
        None,
        TextureUsage::RENDER_ATTACHMENT | TextureUsage::SAMPLED,
        self.0.format,
        size,
        1,
      );
      let view = core.create_texture_view(texture.clone());
      *active = (size, texture, view);
    }

    // Output
    outputs[Self::OUTPUT_TEXTURE] = Some(FrameNodeValue::TextureView(active.2.clone()));
  }
}
