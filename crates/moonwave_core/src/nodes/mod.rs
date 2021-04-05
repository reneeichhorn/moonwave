use crate::Core;
use moonwave_common::Vector2;
use moonwave_render::{CommandEncoder, CommandEncoderOutput, FrameGraphNode, FrameNodeValue};
use moonwave_resources::*;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use shaderc::ShaderKind;
use std::sync::Arc;

static PRESENT_TO_SCREEN_PROGRAM: OnceCell<PresentToScreenResources> = OnceCell::new();

pub struct PresentToScreen {}

struct PresentToScreenResources {
  _vs: ResourceRc<Shader>,
  _fs: ResourceRc<Shader>,
  _pipeline_layout: ResourceRc<PipelineLayout>,
  pipeline: ResourceRc<RenderPipeline>,
}

impl PresentToScreen {
  pub const INPUT_TEXTURE: usize = 0;
  pub const INPUT_TEXTURE_UI: usize = 1;

  pub fn new() -> Self {
    let _ = PRESENT_TO_SCREEN_PROGRAM.get_or_init(|| {
      let core = Core::get_instance();
      let vs = core
        .create_shader_from_glsl(
          include_str!("./passthrough.vert"),
          "PassthroughVS",
          ShaderKind::Vertex,
        )
        .unwrap();

      let fs = core
        .create_shader_from_glsl(
          include_str!("./passthrough.frag"),
          "PassthroughFS",
          ShaderKind::Fragment,
        )
        .unwrap();

      let layout_desc = PipelineLayoutDescriptor::new().add_binding(
        core
          .get_gp_resources()
          .sampled_texture_bind_group_layout
          .clone(),
      );
      let pipeline_layout = core.create_pipeline_layout(layout_desc);

      let pipeline_desc = RenderPipelineDescriptor::new_without_vertices(
        pipeline_layout.clone(),
        vs.clone(),
        fs.clone(),
      )
      .add_color_output(TextureFormat::Bgra8UnormSrgb);
      let pipeline = core.create_render_pipeline(pipeline_desc);

      PresentToScreenResources {
        _vs: vs,
        _fs: fs,
        _pipeline_layout: pipeline_layout,
        pipeline,
      }
    });

    PresentToScreen {}
  }
}

impl FrameGraphNode for PresentToScreen {
  fn execute_raw(
    &self,
    inputs: &[Option<FrameNodeValue>],
    _outputs: &mut [Option<FrameNodeValue>],
    device: &wgpu::Device,
    _queue: &wgpu::Queue,
    sc_frame: &wgpu::SwapChainFrame,
  ) -> CommandEncoderOutput {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
      label: Some("CommandEncoderPresentToScreen"),
    });

    {
      let resources = PRESENT_TO_SCREEN_PROGRAM.get().unwrap();
      let pipeline = resources.pipeline.get_raw();

      let bind_groups = inputs
        .iter()
        .filter_map(|input| {
          if let Some(FrameNodeValue::SampledTexture(texture)) = input {
            Some(texture.bind_group.get_raw())
          } else {
            None
          }
        })
        .collect::<Vec<_>>();

      {
        let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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

        for bind_group in bind_groups.iter() {
          rp.set_pipeline(&*pipeline);
          rp.set_bind_group(0, &*bind_group, &[]);
          rp.draw(0..4, 0..1);
        }
      }
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
  active: Arc<Mutex<(Vector2<u32>, SampledTexture, bool)>>,
}

impl TextureGeneratorHost {
  pub fn new(size: TextureSize, format: TextureFormat) -> Arc<Self> {
    let core = Core::get_instance();
    let actual_size = size.get_actual_size();
    let texture = core.create_sampled_texture(
      None,
      TextureUsage::RENDER_ATTACHMENT | TextureUsage::SAMPLED,
      format,
      actual_size,
      1,
    );

    Arc::new(Self {
      format,
      size,
      active: Arc::new(Mutex::new((actual_size, texture, false))),
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

    let active_cloned = self.0.active.clone();
    let mut active = self.0.active.lock();
    if size != active.0 && !active.2 {
      let core = Core::get_instance();
      active.2 = true;
      let format = self.0.format;

      core.spawn_background_task(move || {
        /*
        let texture = core.create_sampled_texture(
          None,
          TextureUsage::RENDER_ATTACHMENT | TextureUsage::SAMPLED,
          format,
          size,
          1,
        );
        let mut active = active_cloned.lock();
        active.0 = size;
        active.1 = texture;
        active.2 = false;
        */
      });
    }

    // Output
    outputs[Self::OUTPUT_TEXTURE] = Some(FrameNodeValue::SampledTexture(active.1.clone()));
  }
}
