use proc_macro::*;
use proc_macro_error::*;
use syn::parse_macro_input;

mod core;
mod render;
use crate::core::*;

#[proc_macro_attribute]
#[proc_macro_error]
pub fn component(_attr: TokenStream, item: TokenStream) -> TokenStream {
  let attribute = parse_macro_input!(item as ComponentAttribute);
  let gen = attribute.generate_code();
  println!("{}", gen);
  gen
}

#[proc_macro]
pub fn block(_item: TokenStream) -> TokenStream {
  TokenStream::new()
}
