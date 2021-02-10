pub fn invert_option_result<T, E>(opt: Option<Result<T, E>>) -> Result<Option<T>, E> {
  opt.map_or(Ok(None), |v| v.map(Some))
}
