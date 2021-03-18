use moonwave_core::Core;

pub struct StableFrameValue;

impl StableFrameValue {
  #[inline]
  pub fn get_elapsed_micros() -> u64 {
    Core::get_instance().get_elapsed_time()
  }

  #[inline]
  pub fn based_on_second(target_value: f32) -> f32 {
    let elapsed_micros = Self::get_elapsed_micros();
    (target_value / 1000000.0) * elapsed_micros as f32
  }
}
