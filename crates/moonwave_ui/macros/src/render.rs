use std::collections::HashMap;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::{
  analyze::AnalyzedStatement,
  render_parse::{RenderComponent, RenderComponentChildren},
  AnalyzedMethod,
};

#[derive(Debug)]
pub struct RenderMethod {
  pub(crate) method: AnalyzedMethod,
  pub storage: HashMap<String, RenderStorageEntry>,
}

impl RenderMethod {
  pub(crate) fn new(method: AnalyzedMethod) -> Self {
    let mut new = Self {
      method: method.clone(),
      storage: HashMap::new(),
    };
    for (index, stmt) in method.stmts.iter().enumerate() {
      match stmt {
        AnalyzedStatement::RenderItem(render) => new.add_storage(render, format!("stmt_{}", 0)),
        _ => {}
      }
    }
    new
  }

  fn add_storage(&mut self, c: &RenderComponent, prefix: String) {
    self
      .storage
      .insert(prefix.clone(), RenderStorageEntry::SingleTyped);

    match &c.children {
      RenderComponentChildren::Fixed(children) => {
        for (index, child) in children.iter().enumerate() {
          if child.parent {
            continue;
          }
          self.add_storage(child, format!("{}_{}", prefix, index));
        }
      }
      _ => {}
    }
  }

  pub fn build_struct(&self) -> TokenStream {
    let items = self.storage.iter().map(|(name, ty)| {
      let ident = format_ident!("{}", name);
      quote! {
        #ident: Option<moonwave_ui::HostedComponentRc>
      }
    });
    quote! {
      #[derive(Default)]
      struct Storage {
        #(#items),*
      }
    }
  }

  fn build_create(&self) -> TokenStream {
    let tokens = self.method.stmts.iter().map(|stmt| stmt.build_code());
    quote! {
      let mut out = None;
      #(#tokens)*
      out
    }
  }

  pub fn build_impl(&self, name: &syn::Type) -> TokenStream {
    let create = self.build_create();
    quote! {
      impl moonwave_ui::Component for #name {
        fn get_layout_props(&self) -> &LayoutProps {
          &self.layout
        }

        fn get_layout_props_mut(&mut self) -> &mut LayoutProps {
          &mut self.layout
        }

        fn create(&mut self, alloc: &mut moonwave_ui::Allocator) -> Option<ChildrenProxy> {
          #create
        }

        fn update(&mut self, updates: Box<dyn moonwave_ui::UpdateList>) {
        }

        fn offer_layout(&self, size: (f32, f32)) -> (f32, f32) {
          let layouter = moonwave_ui::DefaultLayouter::new(self.storage.stmt_0.as_ref().unwrap().clone());
          layouter.handle_offering(size)
        }

        fn mount(&mut self, size: (f32, f32), position: (f32, f32)) {
          let mut root = std::cell::RefCell::borrow_mut(self.storage.stmt_0.as_ref().unwrap());
          root.component.mount(size, position);
        }
      }
    }
  }
}

#[derive(Debug)]
pub enum RenderStorageEntry {
  SingleTyped,
  VecTyped,
}
