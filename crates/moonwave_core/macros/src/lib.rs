use proc_macro::*;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::{
  parenthesized,
  parse::{Parse, ParseStream},
  parse2, Block, FnArg, ImplItem, ImplItemMethod, ItemImpl, LitInt, Type,
};
use syn::{parse_macro_input, ItemStruct, Result, Token};

enum Item {
  Struct(ItemStruct),
  Impl(ItemImpl),
}

impl Parse for Item {
  fn parse(input: ParseStream) -> Result<Self> {
    let lookahead = input.lookahead1();
    if lookahead.peek(Token![struct]) {
      input.parse().map(Item::Struct)
    } else if lookahead.peek(Token![impl]) {
      input.parse().map(Item::Impl)
    } else {
      Err(lookahead.error())
    }
  }
}

impl Item {
  fn generate(&self) -> TokenStream2 {
    match self {
      Item::Struct(strct) => {
        let ident = strct.ident.clone();
        let sub_ident = format_ident!("{}Actor", strct.ident.clone());
        let fields = strct.fields.iter().collect::<Vec<_>>();
        let field_names = strct
          .fields
          .iter()
          .map(|f| f.ident.clone().unwrap())
          .collect::<Vec<_>>();

        let x = quote! {
          #strct

          #[doc(hidden)]
          struct #sub_ident {
            #(#fields,)*
            ext: moonwave_core::ActorBaseExt,
          }

          impl moonwave_core::IntoActor<#sub_ident> for #ident {
            fn into_actor(self, core: std::sync::Arc<moonwave_core::Core>) -> #sub_ident {
              #sub_ident {
                #(#field_names: self.#field_names,)*
                ext: moonwave_core::ActorBaseExt::new(core),
              }
            }
          }
        };
        println!("xxx {}", x);
        x
      }
      Item::Impl(im) => {
        let ident = match &*im.self_ty {
          Type::Path(path) => path.path.get_ident().unwrap(),
          _ => panic!("Unknown implementation ident"),
        };
        let sub_ident = format_ident!("{}Actor", ident.clone());

        //let mut ticks = Vec::new();
        let mut timers = Vec::new();
        let mut items = Vec::new();

        for item in &im.items {
          match item {
            ImplItem::Method(method) => {
              for attr in &method.attrs {
                let name = attr.path.get_ident().unwrap().to_string();
                #[allow(clippy::single_match)]
                match name.as_str() {
                  "actor_tick" => {
                    let x = attr.tokens.clone().into_iter().next().unwrap();
                    if let proc_macro2::TokenTree::Group(g) = x {
                      let ty = parse2::<TickType>(g.stream()).unwrap();
                      timers.push(Tick {
                        ty,
                        method: ActorMethod::new(method),
                      });
                    }
                  }
                  _ => {}
                }
              }
            }
            _ => {
              items.push(item.clone());
            }
          }
        }

        let timer_setup = timers
          .iter()
          .filter(|tick| matches!(tick.ty, TickType::Timer { .. }))
          .map(|tick| {
            let every_ms: u64 = match tick.ty {
              TickType::Timer(TimerValue::Seconds(x)) => x as u64 * 1000,
              TickType::Timer(TimerValue::Milliseconds(x)) => x as u64,
              _ => 0,
            };
            quote! {{
              self.ext.timers.push(moonwave_core::Timer {
                every_ms: #every_ms,
                elapsed: 0,
                dirty: false,
              });
            }}
          });

        let (optional_tick_system, optional_tick_register) = if !timers.is_empty() {
          let tick_system_ident = format_ident!("{}TickSystem", ident.clone());
          let registered_ident = format_ident!("{}TickSystemRegistered", ident.clone());

          let needs_mutability = timers.iter().any(|t| t.method.needs_mutability);
          let tick_execs = timers
            .iter()
            .filter(|tick| tick.ty == TickType::Real)
            .map(|tick| tick.method.body.clone());

          let timer_execs = timers
            .iter()
            .filter(|tick| matches!(tick.ty, TickType::Timer { .. }))
            .enumerate()
            .map(|(id, tick)| {
              let body = tick.method.body.clone();
              quote! {
                {
                  let mut timer = &mut self.ext.timers[#id];
                  if timer.dirty {
                    timer.dirty = false;
                    #body
                  }
                }
              }
            });

          let mut tick_signature = TokenStream2::new();
          let mut tick_query = TokenStream2::new();
          let tick_params = TokenStream2::new();
          if needs_mutability {
            tick_signature.extend(quote! { &mut self, });
            tick_query.extend(quote! { &mut #sub_ident, });
          } else {
            tick_signature.extend(quote! { &self, });
            tick_query.extend(quote! { &#sub_ident, });
          }

          (
            Some(quote! {
              #[doc(hidden)]
              #[allow(non_upper_case_globals)]
              static #registered_ident: std::sync::Once = std::sync::Once::new();
              #[doc(hidden)]
              struct #tick_system_ident;

              impl moonwave_core::System for #tick_system_ident {
                fn execute_system(&mut self, core: std::sync::Arc<moonwave_core::Core>, elapsed: u64) {
                  let world = core.get_world();
                  let mut query = world.query::<(#tick_query)>();
                  for (_entity, (actor, #tick_params)) in query.iter() {
                    actor.tick(#tick_params elapsed);
                  }
                }
              }

              impl #sub_ident {
                pub fn tick(#tick_signature elapsed: u64) {
                  // Execute ticks.
                  #(#tick_execs)*

                  // Tick timers
                  self.ext.tick(elapsed);

                  // Execute timers.
                  #(#timer_execs)*
                }
              }
            }),
            Some(quote! {
              #registered_ident.call_once(|| {
                world.add_system::<#tick_system_ident>(#tick_system_ident {});
              });
            }),
          )
        } else {
          (None, None)
        };

        quote! {
          // Tick System if used
          #optional_tick_system


          // Actor trait implementation.
          impl moonwave_core::Actor for #sub_ident {
            fn get_actor_ext(&self) -> &moonwave_core::ActorBaseExt {
              &self.ext
            }

            fn get_actor_ext_mut(&mut self) -> &mut moonwave_core::ActorBaseExt {
              &mut self.ext
            }

            fn into_raw_entity(mut self) -> moonwave_core::Entity {
              // Setup timers.
              #(#timer_setup)*

              // Attach data to actor.
              let world = self.ext.core.get_world().clone();
              let entity = world.reserve();
              self.ext.entity = entity;

              // Create entity.
              let mut builder = moonwave_core::EntityBuilder::new();
              builder.add(self);
              world.spawn_at(entity, builder);

              // Setup system
              #optional_tick_register

              entity
            }
          }
        }
      }
    }
  }
}

#[proc_macro_attribute]
pub fn actor(_attr: TokenStream, item: TokenStream) -> TokenStream {
  let item = parse_macro_input!(item as Item);
  TokenStream::from(item.generate())
}

#[derive(Debug, PartialEq, Eq)]
enum TimerValue {
  Seconds(usize),
  Milliseconds(usize),
}

#[derive(Debug, PartialEq, Eq)]
enum TickType {
  Real,
  Timer(TimerValue),
}

struct Tick {
  ty: TickType,
  method: ActorMethod,
}

struct ActorMethod {
  body: Block,
  uses_self: bool,
  needs_mutability: bool,
}

impl ActorMethod {
  pub fn new(method: &ImplItemMethod) -> Self {
    let method_self = match method
      .sig
      .inputs
      .iter()
      .find(|input| matches!(input, FnArg::Receiver(..)))
    {
      Some(FnArg::Receiver(rec)) => Some(rec.clone()),
      _ => None,
    };

    Self {
      body: method.block.clone(),
      uses_self: method_self.is_some(),
      needs_mutability: method_self
        .map(|m| m.mutability.is_some())
        .unwrap_or_default(),
    }
  }
}

impl Parse for TickType {
  fn parse(input: ParseStream) -> Result<Self> {
    let ident = input.parse::<syn::Ident>()?;
    match ident.to_string().as_str() {
      "real" => Ok(TickType::Real),
      "timer" => {
        let content;
        parenthesized!(content in input);
        let value = content.parse::<LitInt>()?;
        match value.suffix() {
          "s" => Ok(TickType::Timer(TimerValue::Seconds(
            value.base10_parse().unwrap(),
          ))),
          "ms" => Ok(TickType::Timer(TimerValue::Milliseconds(
            value.base10_parse().unwrap(),
          ))),
          _ => Err(syn::Error::new(
            Span::call_site(),
            "Unexpected timer value variant (only ms and s allowed)",
          )),
        }
      }
      _ => Err(syn::Error::new(
        Span::call_site(),
        "Unexpected timer variant (only 'real' and 'timer' are allowed)",
      )),
    }
  }
}

#[proc_macro_attribute]
pub fn actor_tick(_attr: TokenStream, _item: TokenStream) -> TokenStream {
  TokenStream::new()
}
