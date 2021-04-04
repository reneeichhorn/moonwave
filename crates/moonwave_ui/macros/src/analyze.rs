use proc_macro2::TokenStream;
use quote::quote;
use syn::{token::Semi, Expr, Item, Local, Stmt};

use crate::render_parse::RenderComponent;

#[derive(Clone, Debug)]
pub enum AnalyzedStatement {
  Local(Local),
  Item(Item),
  RenderItem(RenderComponent),
  Expr(AnalyzedExpression),
  Semi(AnalyzedExpression, Semi),
}

#[derive(Clone, Debug)]
pub enum AnalyzedExpression {
  Default(Expr),
  RenderExpr(RenderComponent),
}

impl AnalyzedStatement {
  pub fn new(stmt: &Stmt) -> Self {
    match stmt {
      Stmt::Local(local) => AnalyzedStatement::Local(local.clone()),
      Stmt::Item(item) => {
        if let Item::Macro(mac) = item {
          let macro_name = mac.mac.path.get_ident().as_ref().unwrap().to_string();
          if macro_name == "render" {
            return AnalyzedStatement::RenderItem(RenderComponent::new(&mac.mac));
          }
        }
        AnalyzedStatement::Item(item.clone())
      }
      Stmt::Expr(expr) => AnalyzedStatement::Expr(AnalyzedExpression::new(expr)),
      Stmt::Semi(expr, semi) => AnalyzedStatement::Semi(AnalyzedExpression::new(expr), *semi),
    }
  }

  pub fn build_code(&self) -> TokenStream {
    match &self {
      AnalyzedStatement::Semi(expr, semi) => {
        let expr = expr.build_code();
        quote! {
          #expr #semi
        }
      }
      AnalyzedStatement::Expr(expr) => expr.build_code(),
      AnalyzedStatement::Item(item) => quote! { #item },
      AnalyzedStatement::Local(local) => quote! { #local},
      AnalyzedStatement::RenderItem(render) => {
        let name = "stmt_0".to_string();
        render.build_code(name, None)
      }
    }
  }
}

impl AnalyzedExpression {
  pub fn new(expr: &Expr) -> Self {
    match expr {
      Expr::Macro(mac) => {
        panic!("Macro expression not allowed yet")
      }
      _ => AnalyzedExpression::Default(expr.clone()),
    }
  }

  pub fn build_code(&self) -> TokenStream {
    match &self {
      AnalyzedExpression::Default(expr) => quote! { #expr },
      _ => panic!("Unimplemented expression type for render code"),
    }
  }
}
