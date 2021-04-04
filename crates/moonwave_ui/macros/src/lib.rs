use std::collections::HashMap;

use proc_macro::TokenStream;
use quote::quote;
use syn::{
  parse::{Parse, ParseStream},
  parse_macro_input, Ident, ImplItem, ImplItemMethod, Item, ItemImpl, Local, Result, Type,
};

mod analyze;
mod render;
mod render_parse;

use analyze::AnalyzedStatement;
use render::RenderMethod;

struct ComponentImpl {
  name: Type,
  generic_methods: HashMap<String, AnalyzedMethod>,
  render_method: RenderMethod,
}

impl Parse for ComponentImpl {
  fn parse(input: ParseStream) -> Result<Self> {
    let implementation = input.parse::<ItemImpl>()?;
    let mut generic_methods = HashMap::new();
    let mut render_method = None;

    for item in implementation.items {
      match item {
        ImplItem::Method(method) => {
          let analyzed = AnalyzedMethod::new(&method);

          // Special logic for render method.
          if analyzed.name == "render" {
            render_method = Some(RenderMethod::new(analyzed.clone()));
          } else {
            generic_methods.insert(analyzed.name.clone(), analyzed);
          }
        }
        _ => panic!("Unexpected impl item only methods are allowed"),
      }
    }

    Ok(ComponentImpl {
      name: *implementation.self_ty.clone(),
      generic_methods,
      render_method: render_method
        .unwrap_or_else(|| panic!("Component implementation must contain a render method")),
    })
  }
}

#[derive(Clone, Debug)]
struct AnalyzedMethod {
  name: String,
  self_usage: SelfUsage,
  method: ImplItemMethod,
  stmts: Vec<AnalyzedStatement>,
}

impl AnalyzedMethod {
  pub fn new(item: &ImplItemMethod) -> Self {
    Self {
      name: item.sig.ident.to_string(),
      self_usage: SelfUsage::None,
      method: item.clone(),
      stmts: item
        .block
        .stmts
        .iter()
        .map(AnalyzedStatement::new)
        .collect::<Vec<_>>(),
    }
  }
}

#[derive(Copy, Clone, Debug)]
enum SelfUsage {
  None,
  Immutable,
  Mutable,
}

#[proc_macro_attribute]
pub fn component(_attr: TokenStream, item: TokenStream) -> TokenStream {
  let component = parse_macro_input!(item as ComponentImpl);
  let storage_struct = component.render_method.build_struct();
  let component_impl = component.render_method.build_impl(&component.name);
  let component_name = &component.name;
  let fns = component
    .generic_methods
    .iter()
    .map(|(_, x)| x.method.clone());

  TokenStream::from(quote! {
    #storage_struct
    #component_impl
    impl #component_name {
      #(#fns)*
    }
  })
}

#[proc_macro_attribute]
pub fn render(_attr: TokenStream, item: TokenStream) -> TokenStream {
  TokenStream::new()
}
