use bytemuck::{Pod, Zeroable};

use super::{AsStd140, Std140};

#[allow(missing_docs)]
#[derive(Debug, Clone, Copy)]
pub struct Array<T: Std140, const N: usize> {
  pub elements: [T; N],
}

unsafe impl<T: Std140, const N: usize> Zeroable for Array<T, N> {}
unsafe impl<T: Std140, const N: usize> Pod for Array<T, N> {}

unsafe impl<T: Std140, const N: usize> Std140 for Array<T, N> {
  const ALIGNMENT: usize = 4;
}

impl<T: AsStd140, const N: usize> AsStd140 for [T; N] {
  type Std140Type = Array<T::Std140Type, N>;
  fn as_std140(&self) -> Self::Std140Type {
    let mut array = Array::<T::Std140Type, N>::zeroed();
    #[allow(clippy::needless_range_loop)]
    for i in 0..N {
      array.elements[i] = self[i].as_std140();
    }
    array
  }
}
