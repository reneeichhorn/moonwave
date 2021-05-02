use std::sync::Arc;

/// Safety: Only safe within the actor spawn context. Will result in undefined behavior if used elsewhere.
pub struct TemporalClone<T> {
  value: Option<Arc<T>>,
}

impl<T> TemporalClone<T> {
  pub fn new(value: T) -> Self {
    Self {
      value: Some(Arc::new(value)),
    }
  }

  pub fn take(mut self) -> Arc<T> {
    self.value.take().unwrap()
  }
}

impl<T> Clone for TemporalClone<T> {
  fn clone(&self) -> Self {
    unsafe {
      let ptr = self as *const Self;
      let mutable = ptr as *mut Self;
      Self {
        value: (*mutable).value.take(),
      }
    }
  }
}
