use crate::Core;
use std::sync::Arc;

pub trait Extension: Send + Sync + 'static {
  fn before_tick(&self, core: Arc<Core>);
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

  pub fn before_tick(&self, core: Arc<Core>) {
    for ext in &self.extensions {
      ext.before_tick(core.clone());
    }
  }
}
