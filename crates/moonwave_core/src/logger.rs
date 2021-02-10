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
    if target.contains("gfx") {
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
  let logger = log::set_boxed_logger(Box::new(Logger));
  log::set_max_level(LevelFilter::Debug);
  logger.unwrap();
}
