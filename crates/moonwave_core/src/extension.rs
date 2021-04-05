pub trait Extension: Send + Sync + 'static {
  fn before_tick(&self);
}

pub(crate) struct ExtensionHost {
  extensions: Vec<Box<dyn Extension>>,
}

impl ExtensionHost {
  pub(crate) fn new() -> Self {
    Self {
      extensions: Vec::new(),
    }
  }

  pub fn add<T: Extension>(&mut self, extension: T) {
    self.extensions.push(Box::new(extension));
  }

  pub fn before_tick(&self) {
    for ext in &self.extensions {
      ext.before_tick();
    }
  }
}
