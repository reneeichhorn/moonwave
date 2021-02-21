use moonwave_render::{CommandEncoderOutput, FrameGraphNode, FrameNodeValue};
pub struct PresentToScreen {}

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
