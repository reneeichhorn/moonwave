use moonwave_core_macro::*;

#[actor]
struct MyTestActor {
  number: usize,
}

#[actor]
impl MyTestActor {
  #[actor_tick(real)]
  fn tick(&mut self) {
    self.number += 2;
  }

  #[actor_tick(timer(1s))]
  fn tick_every_second(&mut self) {
    self.number += 1;
  }
}

#[test]
pub fn basic_test() {
  let x = 1usize.min(2);
  assert!(x >= 1);
}
