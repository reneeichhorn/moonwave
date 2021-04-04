use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
  braced,
  parse::{discouraged::Speculative, Parse, ParseStream},
  parse_quote, Expr, ExprCall, ExprMethodCall, ExprPath, ExprRange, Macro, Result, Token,
};

#[derive(Clone, Debug)]
pub struct RenderComponent {
  pub expr: Expr,
  pub children: RenderComponentChildren,
  pub parent: bool,
}

#[derive(Clone, Debug)]
pub enum RenderComponentChildren {
  Fixed(Vec<RenderComponent>),
}

impl RenderComponent {
  pub fn new(mac: &Macro) -> Self {
    let body = mac.parse_body();
    body.unwrap()
  }

  pub fn build_code(&self, name: String, with_parent: Option<String>) -> TokenStream {
    let expr = &self.expr;
    let storage_ident = format_ident!("{}", name);
    if self.parent {
      let parent_name = &name[..name.rfind('_').unwrap()];
      let parent_ident = format_ident!("{}", parent_name);
      return quote! {
        out = Some(moonwave_ui::ChildrenProxy::new(self.storage.#parent_ident.as_ref().unwrap().clone()));
      };
    }

    let children = match &self.children {
      RenderComponentChildren::Fixed(children) => children
        .iter()
        .enumerate()
        .map(|(index, child)| child.build_code(format!("{}_{}", name, index), Some(name.clone()))),
    };

    let parent_assign = if let Some(parent) = with_parent {
      let name = format_ident!("{}", parent);
      quote! {
        std::cell::RefCell::borrow_mut(self.storage.#name.as_ref().unwrap()).add_child(self.storage.#storage_ident.as_ref().unwrap().clone());
      }
    } else {
      TokenStream::new()
    };

    quote! {
      self.storage.#storage_ident = Some(alloc.alloc(#expr));
      #parent_assign
      #(#children)*
    }
  }
}

impl Parse for RenderComponent {
  fn parse(input: ParseStream) -> Result<Self> {
    let mut parent = false;
    let expr = if let Ok(e) = try_parse::<ExprMethodCall>(&input) {
      Expr::MethodCall(e)
    } else if let Ok(e) = try_parse::<ExprCall>(&input) {
      let func = &e.func;
      let args = &e.args;
      let expr = parse_quote! { #func::new(#args) };
      expr
    } else if let Ok(e) = try_parse::<ExprPath>(&input) {
      let expr = parse_quote! { #e::new() };
      expr
    } else if let Ok(_e) = try_parse::<ExprRange>(&input) {
      if parent {
        panic!("Children are placed at multiple areas in render tree.");
      }
      parent = true;
      parse_quote! {
        let _ = 1
      }
    } else {
      panic!("Unsupported expression in render tree {:#?}", input)
    };

    let children = if input.peek(syn::token::Brace) {
      let content;
      braced!(content in input);

      let block = content.parse_terminated::<RenderComponent, Token![,]>(RenderComponent::parse);
      block.unwrap().into_iter().collect::<Vec<_>>()
    } else {
      Vec::new()
    };

    Ok(Self {
      expr,
      parent,
      children: RenderComponentChildren::Fixed(children),
    })
  }
}

fn try_parse<P: Parse>(input: &ParseStream) -> Result<P> {
  let fork = input.fork();
  let parsed = fork.parse::<P>();
  if parsed.is_ok() {
    input.advance_to(&fork);
  }
  parsed
}
