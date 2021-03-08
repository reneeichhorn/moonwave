use chrono::*;

pub use log::*;

struct Logger;

impl log::Log for Logger {
  fn enabled(&self, metadata: &Metadata) -> bool {
    metadata.level() <= Level::Debug
  }

  fn log(&self, record: &Record) {
    // Filter based on level.
    if !self.enabled(record.metadata()) {
      return;
    }

    // Filter modules
    let target = record.target().to_string();
    let level = record.level();
    if target.contains("gfx") || target.contains("naga") {
      return;
    }
    /*
    if level != Level::Error
      && level != Level::Warn
      && (target.contains("wgpu") || target.contains("gfx"))
    {
      return;
    }
    */

    let message = format!("{}", record.args());
    let now: DateTime<Utc> = Utc::now();
    let time = now.format("%H:%M:%S%.6f").to_string();

    println!("{} [{}] [{}] {}", time, level, target, message);
  }

  fn flush(&self) {}
}

pub(crate) fn init() {
  start_deadlock_detection();
  let logger = log::set_boxed_logger(Box::new(Logger));
  log::set_max_level(LevelFilter::Debug);
  logger.unwrap();
}

use parking_lot::deadlock;

fn start_deadlock_detection() {
  // Create a background thread which checks for deadlocks every 10s
  std::thread::spawn(move || loop {
    std::thread::sleep(std::time::Duration::from_secs(10));
    let deadlocks = deadlock::check_deadlock();
    if deadlocks.is_empty() {
      continue;
    }

    println!("{} deadlocks detected", deadlocks.len());
    for (i, threads) in deadlocks.iter().enumerate() {
      println!("Deadlock #{}", i);
      for t in threads {
        println!("Thread Id {:#?}", t.thread_id());
        println!("{:#?}", t.backtrace());
      }
    }
  });
}
