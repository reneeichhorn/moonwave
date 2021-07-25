use std::collections::VecDeque;

pub struct SharedSimpleBuffer {
  free: VecDeque<usize>,
}

impl SharedSimpleBuffer {
  pub fn new(chunks: usize) -> Self {
    Self {
      free: (0..chunks).collect::<VecDeque<_>>(),
    }
  }

  pub fn alloc(&mut self) -> Option<SharedSimpleBufferAllocation> {
    self
      .free
      .pop_front()
      .map(|index| SharedSimpleBufferAllocation { index })
  }

  pub fn free(&mut self, allocation: SharedSimpleBufferAllocation) {
    self.free.push_back(allocation.index);
  }
}

#[derive(Clone, Debug)]
pub struct SharedSimpleBufferAllocation {
  pub index: usize,
}
