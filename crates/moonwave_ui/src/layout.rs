use std::cell::RefCell;

use crate::HostedComponentRc;

pub enum Alignment {
  Left,
  Center,
  Right,
}

pub struct LayoutProps {
  pub position: (f32, f32),
  pub frame: Option<(f32, f32)>,
  pub spacing: (f32, f32),
  pub alignment: Alignment,
}

impl Default for LayoutProps {
  fn default() -> Self {
    Self {
      position: (0.0, 0.0),
      frame: None,
      spacing: (0.0, 0.0),
      alignment: Alignment::Center,
    }
  }
}

pub struct DefaultLayouter {
  root: HostedComponentRc,
}

impl DefaultLayouter {
  pub fn new(root: HostedComponentRc) -> Self {
    Self { root }
  }

  pub fn handle_offering(&self, size: (f32, f32)) -> (f32, f32) {
    let root = RefCell::borrow_mut(&self.root);
    let layout_props = root.component.get_layout_props();

    // Check wanted frame
    let mut frame = size;
    if let Some(wanted_frame) = layout_props.frame {
      frame = wanted_frame;
    };

    frame
  }
}
