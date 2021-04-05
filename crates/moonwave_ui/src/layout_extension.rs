use crate::Component;

pub trait LayoutExtension: Component + Sized {
  fn frame(mut self, frame: (f32, f32)) -> Self {
    self.get_layout_props_mut().frame = Some(frame);
    self
  }

  fn spacing(mut self, spacing: f32) -> Self {
    self.get_layout_props_mut().spacing = (spacing, spacing);
    self
  }
}

impl<T: Component + Sized> LayoutExtension for T {}
