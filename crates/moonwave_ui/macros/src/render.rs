use heck::CamelCase;
use moonwave_util::invert_option_result;
use proc_macro::*;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote, ToTokens};
use std::cell::RefCell;
use std::rc::Rc;
use syn::parse::{Parse, ParseStream, Parser};
use syn::{
  braced, parenthesized, punctuated::Punctuated, spanned::Spanned, token, Error, Expr, FieldValue,
  Fields, Ident, ImplItemMethod, Item, ItemStruct, Member, Result, Signature, Stmt, Token, Type,
  Visibility,
};

pub struct Property {
  name: Ident,
  value: Expr,
}

pub struct RenderNode {
  name: Ident,
  layout_options: Option<Ident>,
  properties: Vec<Property>,
  children: Vec<RenderNode>,
}

impl Parse for RenderNode {
  fn parse(input: ParseStream) -> Result<Self> {
    // Name
    let ident = input.parse::<Ident>()?;
    let mut layout_options = None;

    // Optional parameters.
    let mut properties = Vec::new();
    let lookahead = input.lookahead1();
    if lookahead.peek(token::Paren) {
      let content;
      let _paren = parenthesized!(content in input);

      // Handle layout options.
      let layout_lh = content.lookahead1();
      if layout_lh.peek(Token![@]) {
        let _ = content.parse::<Token![@]>()?;
        let layout_ident = content.parse::<Ident>()?;
        layout_options = Some(layout_ident);
        let _ = content.parse::<Token![,]>();
      }

      // Handle regular attributes.
      let field_values = Punctuated::<FieldValue, Token![,]>::parse_terminated(&content)?;
      properties.extend(
        field_values
          .into_iter()
          .map(|f| {
            if let Member::Named(named) = f.member {
              return Ok(Property {
                name: named,
                value: f.expr.clone(),
              });
            }
            Err(Error::new(
              f.span(),
              "Invalid field value for a component call",
            ))
          })
          .collect::<Result<Vec<_>>>()?,
      );
    }

    // Optional children.
    let mut children = Vec::new();
    let lookahead = input.lookahead1();
    if lookahead.peek(token::Brace) {
      let content;
      let _brace = braced!(content in input);
      children.extend(Punctuated::<RenderNode, Token![,]>::parse_terminated(&content)?.into_iter())
    }

    Ok(RenderNode {
      name: ident,
      properties,
      layout_options,
      children,
    })
  }
}

fn parse_block(item: &TokenStream2) -> Result<Vec<RenderNode>> {
  let parser = Punctuated::<RenderNode, Token![,]>::parse_terminated;
  let punctuated = parser.parse2(item.clone())?;
  Ok(punctuated.into_iter().collect())
}

#[derive(Clone)]
pub enum Dependency {
  Property(String),
  Variable(String),
}

pub enum DependencyContext {
  SubContext(Vec<DependencyContext>),
}

pub enum RenderGraphNode {
  Block(Rc<DependencyTreeVec>, Vec<RenderNode>),
  GenericExpression(Rc<DependencyTreeVec>, Expr),
  GenericStatement(Rc<DependencyTreeVec>, Stmt),
  IfExpression(
    Rc<DependencyTreeVec>,
    Box<RenderGraphNode>,
    Box<RenderGraphNode>,
    Option<Box<RenderGraphNode>>,
  ),
  RetExpression(Rc<DependencyTreeVec>, Option<Box<RenderGraphNode>>),
  Graph(Rc<DependencyTreeVec>, Vec<RenderGraphNode>),
}

pub struct RenderMethod {
  root_node: RenderGraphNode,
}

pub struct DependencyTreeVec {
  parent: RefCell<Option<Rc<DependencyTreeVec>>>,
  vec: RefCell<Vec<Dependency>>,
}

impl DependencyTreeVec {
  fn new() -> Rc<Self> {
    Rc::new(Self {
      parent: RefCell::new(None),
      vec: RefCell::new(Vec::new()),
    })
  }

  fn push(&self, dependency: Dependency) {
    self.vec.borrow_mut().push(dependency.clone());
    if let Some(parent) = &*self.parent.borrow() {
      parent.push(dependency);
    }
  }

  fn create_child(self: &Rc<Self>) -> Rc<Self> {
    Rc::new(Self {
      parent: RefCell::new(Some(self.clone())),
      vec: RefCell::new(Vec::new()),
    })
  }
}

fn find_expression_dependencies(expr: &Expr, deps: Rc<DependencyTreeVec>) {}

fn parse_graph_expression(expr: &Expr, deps: Rc<DependencyTreeVec>) -> Result<RenderGraphNode> {
  find_expression_dependencies(expr, deps.clone());

  Ok(match expr {
    Expr::Macro(macro_expr) => {
      let ident = macro_expr.mac.path.get_ident().map(|i| i.to_string());
      if Some("block".to_string()) == ident {
        RenderGraphNode::Block(deps, parse_block(&macro_expr.mac.tokens)?)
      } else {
        RenderGraphNode::GenericExpression(deps, expr.clone())
      }
    }
    Expr::If(if_expr) => RenderGraphNode::IfExpression(
      deps.clone(),
      Box::new(parse_graph_expression(&if_expr.cond, deps.create_child())?),
      Box::new(parse_graph_nodes(&if_expr.then_branch.stmts)?),
      if_expr
        .else_branch
        .as_ref()
        .map(|(_, else_expr)| parse_graph_expression(&*else_expr, deps.create_child()))
        .transpose()?
        .map(Box::new),
    ),
    Expr::Return(ret) => RenderGraphNode::RetExpression(
      deps.clone(),
      invert_option_result(
        ret
          .expr
          .as_ref()
          .map(|expr| parse_graph_expression(&*expr, deps.create_child())),
      )?
      .map(Box::new),
    ),
    _ => RenderGraphNode::GenericExpression(deps, expr.clone()),
  })
}

fn parse_graph_nodes(stmts: &[Stmt]) -> Result<RenderGraphNode> {
  let dependencies = DependencyTreeVec::new();
  let mut nodes = Vec::new();
  for stmt in stmts {
    match stmt {
      Stmt::Item(Item::Macro(mac)) => {
        let ident = mac.mac.path.get_ident().map(|i| i.to_string());
        if Some("block".to_string()) == ident {
          nodes.push(RenderGraphNode::Block(
            dependencies.create_child(),
            parse_block(&mac.mac.tokens)?,
          ))
        } else {
          nodes.push(RenderGraphNode::GenericStatement(
            dependencies.create_child(),
            stmt.clone(),
          ))
        }
      }
      Stmt::Expr(expr) => nodes.push(parse_graph_expression(expr, dependencies.create_child())?),
      Stmt::Semi(expr, _) => nodes.push(parse_graph_expression(expr, dependencies.create_child())?),
      Stmt::Local(_) => nodes.push(RenderGraphNode::GenericStatement(
        dependencies.create_child(),
        stmt.clone(),
      )),
      _ => {
        return Err(Error::new(
          stmt.span(),
          "Unsupported statement for a component render function",
        ))
      }
    }
  }
  Ok(RenderGraphNode::Graph(dependencies, nodes))
}

pub fn parse_method(method: ImplItemMethod) -> Result<RenderMethod> {
  // Validation
  if let Some(asyncness) = method.sig.asyncness {
    return Err(Error::new(
      asyncness.span(),
      "A components render method is not allowed to be async",
    ));
  }
  if let Some(constness) = method.sig.constness {
    return Err(Error::new(
      constness.span(),
      "A components render method is not allowed to be const",
    ));
  }
  match method.vis {
    Visibility::Public(_) | Visibility::Crate(_) => {}
    _ => {
      return Err(Error::new(
        method.vis.span(),
        "A components render method must be at least public for the crate",
      ));
    }
  };

  Ok(RenderMethod {
    root_node: parse_graph_nodes(&method.block.stmts)?,
  })
}

fn generate_render_block_code(node: &RenderNode) -> TokenStream2 {
  // Core base
  let name = node.name.clone();

  // Properties
  let props = node.properties.iter().map(|prop| {
    let name = &prop.name;
    let value = &prop.value;
    quote! {
      #name: #value
    }
  });

  // Children
  let children = node.children.iter().map(generate_render_block_code);

  // Layout
  let layout = node.layout_options.as_ref().map(|layout| {
    quote! {
      component.layout_options = Box::new(#layout.clone());
    }
  });

  quote! {{
    // Create component instance
    let mut component = #name {
      #(#props,)*
      ..Default::default()
    };

    // Configure spawned children
    component.children = vec![#(#children,)*];
    #layout

    // Build new arc
    std::sync::Arc::new(std::sync::RwLock::new(component))
  }}
}

fn generate_render_node_code(node: &RenderGraphNode) -> TokenStream2 {
  match node {
    RenderGraphNode::Block(_, block) => {
      let block = block.iter().map(generate_render_block_code);
      quote! { vec![#(#block),*] }
    }
    RenderGraphNode::IfExpression(_, cond, then, elif) => {
      let cond = generate_render_node_code(cond);
      let then = generate_render_node_code(then);
      let elif = if let Some(elif) = elif {
        generate_render_node_code(elif)
      } else {
        TokenStream2::new()
      };
      quote! {
        if #cond {
          #then
        } else {
          #elif
        }
      }
    }
    RenderGraphNode::GenericExpression(_, expr) => {
      quote! { #expr }
    }
    RenderGraphNode::GenericStatement(_, stmt) => {
      quote! { #stmt }
    }
    RenderGraphNode::RetExpression(_, expr) => {
      let expr = expr.as_ref().map(|expr| generate_render_node_code(&*expr));
      quote! { return #expr }
    }
    RenderGraphNode::Graph(_, block) => {
      let block = block
        .iter()
        .map(|b| generate_render_node_code(b))
        .collect::<Vec<_>>();

      quote! {
        #(#block)*
      }
    }
    _ => TokenStream2::new(),
  }
}

pub fn generate_render_code(render_method: &RenderMethod) -> TokenStream2 {
  generate_render_node_code(&render_method.root_node)
}
