use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn component(_attr: TokenStream, item: TokenStream) -> TokenStream {
  //let item = parse_macro_input!(item as Item);
  //TokenStream::from(item.generate())
  item
}
