pub trait Extension: Send + Sync + 'static {
  fn init(&mut self) {}
  fn before_tick(&mut self) {}
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

  pub fn init(&mut self) {
    for ext in &mut self.extensions {
      ext.init();
    }
  }

  pub fn before_tick(&mut self) {
    for ext in &mut self.extensions {
      ext.before_tick();
    }
  }
}
