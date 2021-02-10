use heck::CamelCase;
use proc_macro::*;
use proc_macro2::{Ident as Ident2, TokenStream as TokenStream2};
use quote::{format_ident, quote, ToTokens};
use syn::{
  parse::{Parse, ParseStream},
  Visibility,
};
use syn::{spanned::Spanned, Error, Fields, ImplItem, ItemImpl, ItemStruct, Result, Token, Type};

use crate::render;

pub struct CoreStruct {
  name: Ident2,
  fields: Vec<(Ident2, Type)>,
}

impl CoreStruct {
  pub fn generate_code(self) -> TokenStream {
    // Generate property actions
    let mut actions = Vec::new();
    actions.extend(self.fields.iter().map(|(ident, _ty)| {
      let action_variant_ident = format_ident!("SetProperty{}", ident.to_string().to_camel_case());
      quote! { #action_variant_ident }
    }));
    let actions_ident = format_ident!("{}Action", self.name.to_string());

    // Generate token stream.
    let name = self.name;
    let attributes = self.fields.iter().map(|(name, ty)| {
      let name = format_ident!("{}", *name);
      quote! {
        #name: #ty
      }
    });
    let attributes_default = self.fields.iter().map(|(name, _)| {
      let name = format_ident!("{}", *name);
      quote! {
        #name: Default::default()
      }
    });

    TokenStream::from(quote! {
      pub struct #name {
        #(#attributes,)*
        #[doc(hidden)]
        children: Vec<moonwave_ui::AnyComponentRef>,
        #[doc(hidden)]
        layout_options: Box<dyn std::any::Any>,
        #[doc(hidden)]
        layouter: Box<dyn moonwave_ui::AnyLayouter>,
        layout: moonwave_ui::LayouterResult,
      }

      impl Default for #name {
        fn default() -> Self {
          Self {
            #(#attributes_default,)*
            layout_options: Box::new(moonwave_ui::RelativeLayouterOptions::default()),
            layouter: Box::new(moonwave_ui::RelativeLayouter::default()),
            layout: moonwave_ui::LayouterResult::default(),
            children: Vec::new(),
          }
        }
      }

      #[doc(hidden)]
      pub enum #actions_ident {
        #(#actions,)*
        SetLayout,
        Refresh,
      }
    })
  }
}

impl Parse for CoreStruct {
  fn parse(input: ParseStream) -> Result<Self> {
    let item_struct = input.parse::<ItemStruct>()?;

    // Validation.
    if item_struct.generics.lt_token.is_some() {
      return Err(Error::new(
        item_struct.span(),
        "Components containing generics are not supported yet",
      ));
    }
    match item_struct.vis {
      Visibility::Public(_) | Visibility::Crate(_) | Visibility::Restricted(_) => {}
      _ => {
        return Err(Error::new(
          item_struct.vis.span(),
          "A component must be at least visible at a crate level",
        ));
      }
    };

    // Generate named fields.
    let fields = match &item_struct.fields {
      Fields::Named(named) => named
        .named
        .iter()
        .map(|field| (field.ident.clone().unwrap(), field.ty.clone()))
        .collect::<Vec<_>>(),
      Fields::Unnamed(field) => {
        return Err(Error::new(
          field.span(),
          "Unnamed fields for component are not supported yet",
        ))
      }
      _ => Vec::new(),
    };

    Ok(CoreStruct {
      name: item_struct.ident,
      fields,
    })
  }
}

pub struct Implementation {
  name: String,
  render_method: render::RenderMethod,
}

impl Parse for Implementation {
  fn parse(input: ParseStream) -> Result<Self> {
    let item_impl = input.parse::<ItemImpl>()?;

    // Identify name of component.
    let ident = match &*item_impl.self_ty {
      Type::Path(path) => path.path.get_ident(),
      _ => None,
    };
    let name = if let Some(ident) = ident {
      ident.to_string()
    } else {
      return Err(Error::new(
        item_impl.self_ty.span(),
        "Unsupported self type for component implementation",
      ));
    };

    // Parse functions
    let mut render_method = None;
    for item in &item_impl.items {
      match item {
        ImplItem::Method(method) => match method.sig.ident.to_string().as_str() {
          "render" => {
            render_method = Some(render::parse_method(method.clone())?);
          }
          _ => return Err(Error::new(method.span(), "Unsupported method in component")),
        },
        _ => {
          return Err(Error::new(
            item.span(),
            "Unsupported implementation item for component",
          ));
        }
      }
    }

    // Ensure render method is there
    if render_method.is_none() {
      return Err(Error::new(
        item_impl.span(),
        "Component must have a render method implemented",
      ));
    }

    Ok(Implementation {
      name,
      render_method: render_method.unwrap(),
    })
  }
}

pub enum ComponentAttribute {
  Core(Box<CoreStruct>),
  Implementation(Box<Implementation>),
}

impl Parse for ComponentAttribute {
  fn parse(input: ParseStream) -> Result<Self> {
    let lookahead = input.lookahead1();
    if lookahead.peek(Token![struct]) {
      input.parse().map(ComponentAttribute::Core)
    } else if lookahead.peek(Token![impl]) {
      input.parse().map(ComponentAttribute::Implementation)
    } else if lookahead.peek(Token![pub]) {
      input.parse().map(ComponentAttribute::Core)
    } else {
      Err(lookahead.error())
    }
  }
}

impl ComponentAttribute {
  pub fn generate_code(self) -> TokenStream {
    match self {
      ComponentAttribute::Core(core) => core.generate_code(),
      ComponentAttribute::Implementation(core_impl) => {
        let struct_name = format_ident!("{}", core_impl.name);
        let actions_name = format_ident!("{}Action", core_impl.name);
        let render = render::generate_render_code(&core_impl.render_method);

        TokenStream::from(quote! {
          impl moonwave_ui::Component for #struct_name {
            fn handle_action(&mut self, action: Box<dyn std::any::Any>) {
              let _action = action.downcast_ref::<#actions_name>().unwrap();
            }

            #[allow(clippy::needless_update)]
            fn full_render(&self) -> Vec<moonwave_ui::AnyComponentRef> {
              #render
            }

            fn layout(&mut self, parent: &moonwave_ui::LayouterResult) {
              // Create base layout
              let self_layout = self.layouter.evaluate(&self.layout_options, parent);
              self.layout = self_layout;

              // Layout children
              for child in &self.children {
                let mut child = child.write().unwrap();
                child.layout(&self.layout);
              }
            }

            fn mount(&self) {
              for child in &self.children {
                let mut child = child.read().unwrap();
                child.mount();
              }
            }
          }
        })
      }
    }
  }
}
