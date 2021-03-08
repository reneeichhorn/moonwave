use rayon::{ThreadPool, ThreadPoolBuilder};

pub struct Execution {
  frame_thread_pool: ThreadPool,
  background_thread_pool: ThreadPool,
}

impl Execution {
  pub fn new(size: usize) -> Self {
    let frame_thread_pool = ThreadPoolBuilder::new()
      .num_threads(size)
      .thread_name(|i| format!("Frame Worker {}", i))
      .start_handler(|i| {
        optick::register_thread(format!("Frame Worker {}", i).as_str());
      })
      .build()
      .unwrap();

    let background_thread_pool = ThreadPoolBuilder::new()
      .num_threads(size)
      .thread_name(|i| format!("Background Worker {}", i))
      .start_handler(|i| {
        optick::register_thread(format!("Background Worker {}", i).as_str());
      })
      .build()
      .unwrap();

    Execution {
      frame_thread_pool,
      background_thread_pool,
    }
  }

  #[inline]
  pub fn get_frame_thread_pool(&self) -> &ThreadPool {
    &self.frame_thread_pool
  }

  #[inline]
  pub fn get_background_thread_pool(&self) -> &ThreadPool {
    &self.background_thread_pool
  }
}
