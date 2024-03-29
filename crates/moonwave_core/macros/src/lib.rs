use heck::*;
use proc_macro::*;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::{
  parenthesized,
  parse::{Parse, ParseStream},
  parse2, parse_quote, FnArg, GenericArgument, ImplItem, ImplItemMethod, ItemImpl, ItemTrait,
  LitInt, Path, PathArguments, TraitItem, Type,
};
use syn::{parse_macro_input, ItemStruct, Result, Token};

enum Item {
  Struct(ItemStruct),
  Impl(ItemImpl),
}

impl Parse for Item {
  fn parse(input: ParseStream) -> Result<Self> {
    let lookahead = input.lookahead1();
    if lookahead.peek(Token![struct]) || lookahead.peek(Token![pub]) {
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
        /*
        let ident = strct.ident.clone();
        let vis = strct.vis.clone();
        let sub_ident = format_ident!("{}Actor", strct.ident.clone());
        let fields = strct.fields.iter().collect::<Vec<_>>();
        let field_names = strct
          .fields
          .iter()
          .map(|f| f.ident.clone().unwrap())
          .collect::<Vec<_>>();
        */

        quote! {
          #strct
        }
      }
      Item::Impl(im) => {
        let ident = match &*im.self_ty {
          Type::Path(path) => path.path.get_ident().unwrap(),
          _ => panic!("Unknown implementation ident"),
        };
        let self_ident_var = format_ident!("{}", ident.to_string().to_snake_case());

        // Timers are tick based functions that are executed every frame
        let mut timers = Vec::new();
        // Spawns are functions that are executed once the base actor has been put into the world.
        let mut spawns = Vec::new();
        // Items are "normal" actor methods.
        let mut items = Vec::new();
        // Event receivers that are hooked for this actor.
        let mut event_receiver = Vec::new();

        'outer: for item in &im.items {
          match item {
            ImplItem::Method(method) => {
              for attr in &method.attrs {
                let name = attr.path.get_ident().unwrap().to_string();
                #[allow(clippy::single_match)]
                match name.as_str() {
                  "actor_spawn" => {
                    let has_attributes = attr.tokens.clone().into_iter().next().is_some();
                    let ty = if has_attributes {
                      SpawnType::Background
                    } else {
                      SpawnType::Blocking
                    };

                    let actor_method = ActorMethod::new(ident.clone(), method);
                    let unlocked_ident = if actor_method.self_usage.mutable {
                      format_ident!("UnlockedSpawnMut")
                    } else {
                      format_ident!("UnlockedSpawn")
                    };
                    spawns.push((ty, actor_method));

                    let mut regular = method.clone();
                    regular.attrs.clear();
                    if ty == SpawnType::Blocking {
                      regular.sig.inputs[0] =
                        parse_quote! { mut self: moonwave_core::#unlocked_ident<'_, Self>};
                    }
                    items.push(ImplItem::Method(regular));
                    continue 'outer;
                  }
                  "actor_tick" => {
                    let x = attr.tokens.clone().into_iter().next().unwrap();
                    if let proc_macro2::TokenTree::Group(g) = x {
                      let ty = parse2::<TickType>(g.stream()).unwrap();
                      timers.push(Tick {
                        ty,
                        method: ActorMethod::new(ident.clone(), method),
                      });
                      let mut regular = method.clone();
                      regular.attrs.clear();
                      items.push(ImplItem::Method(regular));
                    }
                  }
                  "actor_event" => {
                    let has_attributes = attr.tokens.clone().into_iter().next().is_some();
                    let spawn_type = if has_attributes {
                      SpawnType::Background
                    } else {
                      SpawnType::Blocking
                    };

                    let mut item = method.clone();
                    let event_data_input = item.sig.inputs.pop().unwrap();
                    let ty = match event_data_input.value() {
                      FnArg::Typed(ty) => ty.clone(),
                      _ => panic!("Expected event data as last parameter for event receiver."),
                    };
                    let actor_method = ActorMethod::new(ident.clone(), &item);

                    event_receiver.push((item, actor_method, ty, spawn_type));

                    let mut regular = method.clone();
                    regular.attrs.clear();
                    items.push(ImplItem::Method(regular));
                  }
                  _ => {}
                }
              }

              if method.attrs.is_empty() {
                items.push(item.clone());
              }
            }
            _ => {
              items.push(item.clone());
            }
          }
        }

        // Create event receiver
        let (event_receiver_spawn, event_receiver_impl) = if !event_receiver.is_empty() {
          let components = event_receiver.iter().map(|recv| {
            let event_data_type = &recv.2.ty;
            quote! {
              cmd.add_component(entity, moonwave_core::EventReceiver::<#event_data_type>::new());
            }
          });

          let drains = event_receiver.iter().map(|recv| {
            let event_data_type = &recv.2.ty;
            let event_data_var = format_ident!(
              "moonwave_core_event_receiver_{}",
              get_ident_of_type(&event_data_type)
                .to_string()
                .to_snake_case(),
            );
            let event_name = &recv.1.method.sig.ident;
            let vars = recv.1.usages.iter().map(|c| c.name());

            let event_impl = if recv.3 == SpawnType::Background {
              quote! {
                let mut weak = actor.get_weak::<#ident>().unwrap();
                Core::get_instance().spawn_background_task(move || {
                  #[allow(clippy::unnecessary_mut_passed)]
                  #ident::#event_name(&mut weak, #(#vars),* event);
                  weak.flush();
                });
              }
            } else {
              quote! {
                #self_ident_var.#event_name(#(#vars),* event);
              }
            };

            quote! {
              for event in #event_data_var.drain() {
                #event_impl
              }
            }
          });

          (
            Some(quote! {
              #(#components)*
            }),
            Some(quote! {
              #(#drains)*
            }),
          )
        } else {
          (None, None)
        };

        // Create spawn system
        let mut cloned_register = Vec::new();
        let mut before_move = TokenStream2::new();
        let (spawn_system, spawn_system_register) = if !spawns.is_empty() {
          let self_ident = spawns[0].1.self_usage.name();
          let system_ident = format_ident!("{}", format!("{}SpawnSystem", ident).to_snake_case());
          let system_create_ident =
            format_ident!("{}", format!("{}SpawnSystem_system", ident).to_snake_case());
          let event = format!("Actor::{}::spawn", ident.to_string());

          // Collect all used components.
          let (component_streams, query_types, names, rw) =
            ActorMethod::combined(&spawns.iter().map(|(_, s)| s.clone()).collect::<Vec<_>>());

          // Map all spawn functions
          let calls = spawns.iter().filter(|(ty, _)| *ty == SpawnType::Blocking).map(|(_, s)| {
            let unlocked_ident = if s.self_usage.mutable {
              format_ident!("UnlockedSpawnMut")
            } else {
              format_ident!("UnlockedSpawn")
            };
            let names = s.usages.iter().map(|c| c.name());
            let method = s.method.sig.ident.clone();
            quote! {
              {
                let wrapped = moonwave_core::#unlocked_ident::new(&actor, &entity, #self_ident, cmd);
                wrapped.#method (#(#names),*)
              }
            }
          });

          let mut execution_stream = TokenStream2::new();
          let mut register_stream = TokenStream2::new();

          if spawns.iter().any(|(ty, _)| *ty == SpawnType::Blocking) {
            execution_stream.extend(quote! {
              // System
              #[moonwave_core::system]
              #[read_component(moonwave_core::Actor)]
              #component_streams
              fn #system_ident(#[state] new_entity: &moonwave_core::WrappedEntity, world: &mut legion::world::SubWorld, cmd: &mut legion::systems::CommandBuffer) {
                use legion::IntoQuery;
                moonwave_core::optick::event!(#event);

                // Trigger spawn functions
                let mut triggered = false;
                let mut query = <(legion::Entity, &moonwave_core::Actor, #query_types)>::query();
                for (entity, actor, #names) in query.iter_mut(world).filter(|(entity, ..)| *entity == &new_entity.0) {
                  triggered = true;
                  #(#calls)*
                }

                // Reschedule spawn system if not not ready yet.
                if !triggered {
                  moonwave_core::Core::get_instance()
                    .get_world()
                    .add_temp_system(Box::new(#system_create_ident(moonwave_core::WrappedEntity(new_entity.0))));
                }
              }
            });
            register_stream.extend(quote! {
              moonwave_core::Core::get_instance()
                .get_world()
                .add_temp_system(Box::new(#system_create_ident(moonwave_core::WrappedEntity(entity))));
            });
          }

          spawns
            .iter()
            .filter(|(ty, _)| *ty == SpawnType::Background)
            .for_each(|(_, s)| {
              let method = s.method.sig.ident.clone();
              let cloned_params = s.method.sig.inputs.iter().skip(1).map(|input| match input {
                FnArg::Typed(typed) => {
                  let name = get_ident_of_pat(&typed.pat);
                  let name_cloned = format_ident!("{}_cloned", name);
                  let name_cloned_str = name_cloned.to_string();
                  if !cloned_register
                    .iter()
                    .any(|x: &String| x == &name_cloned_str)
                  {
                    before_move.extend(quote! { let #name_cloned = self.#name.clone(); });
                    cloned_register.push(name_cloned.to_string());
                  }

                  quote! {
                    #name_cloned
                  }
                }
                _ => panic!("Self is not allowed in an background spawner"),
              });
              let optick_spawn = format!("Actor::{}::spawn_bg", ident.to_string());
              register_stream.extend(quote! {
                {
                  let core = moonwave_core::Core::get_instance();
                  let mut weak = actor_ref.get_weak();
                  core.spawn_background_task(move || {
                    moonwave_core::optick::event!(#optick_spawn);
                    #rw
                    #[allow(clippy::unnecessary_mut_passed)]
                    #ident::#method(&mut weak, #(#cloned_params),*);
                    weak.flush();
                  });
                }
              });
            });

          (Some(execution_stream), Some(register_stream))
        } else {
          (None, None)
        };

        // Timer setup
        let timer_setup = timers
          .iter()
          .filter(|tick| matches!(tick.ty, TickType::Timer { .. }))
          .map(|tick| {
            let every_micros: u64 = match tick.ty {
              TickType::Timer(TimerValue::Seconds(x)) => x as u64 * 1000 * 1000,
              TickType::Timer(TimerValue::Milliseconds(x)) => x as u64 * 1000,
              _ => 0,
            };
            quote! {{
              timers.push(moonwave_core::Timer {
                every_micros: #every_micros,
                elapsed: 0,
                dirty: false,
              });
            }}
          });

        // Build Tick system and registers
        let (tick_system, tick_register) = if !timers.is_empty() || !event_receiver.is_empty() {
          let event = format!("Actor::{}::tick", ident.to_string());
          let self_ident = self_ident_var.clone();
          let tick_system_ident_register = format_ident!(
            "{}",
            format!("{}TickSystemRegister", ident.to_string()).to_shouty_snake_case()
          );
          let tick_system_ident =
            format_ident!("{}", format!("{}TickSystem", ident).to_snake_case());
          let tick_system_create_ident = format_ident!("{}_system", tick_system_ident);

          let repeated = (0..8).map(|_| quote! { std::sync::Once::new() });

          // Collect all used components.
          let event_componentns = event_receiver.iter().map(|e| e.1.clone());
          let event_recv_components = event_receiver.iter().map(|e| {
            let component = get_path_of_type(&e.2.ty);
            ActorMethod {
              usages: vec![ComponentUsage {
                reader: false,
                writer: false,
                name: get_ident_of_pat(&e.2.pat),
                component: parse_quote! { moonwave_core::EventReceiver<#component> },
                mutable: true,
              }],
              method: event_receiver[0].1.method.clone(),
              self_usage: event_receiver[0].1.self_usage.clone(),
            }
          });
          let (component_streams, query_types, names, rw) = ActorMethod::combined(
            &timers
              .iter()
              .map(|t| t.method.clone())
              .chain(event_componentns)
              .chain(event_recv_components)
              .collect::<Vec<_>>(),
          );

          // Map ticks
          let mut timer_index = 0usize;
          let ticks = timers.iter().map(|t| match &t.ty {
            TickType::Real => {
              let method = t.method.method.sig.ident.clone();
              let names = t.method.usages.iter().map(|c| c.name());
              quote! { #self_ident . #method (#(#names),*) }
            }
            TickType::Timer(_time) => {
              let method = t.method.method.sig.ident.clone();
              let names = t.method.usages.iter().map(|c| c.name());
              let out = quote! {
                let timer = &mut actor.timers[#timer_index];
                timer.tick(elapsed.0);
                if timer.dirty {
                  timer.dirty = false;
                  #self_ident.#method(#(#names),*)
                }
              };
              timer_index += 1;
              out
            }
          });

          (
            Some(quote! {
              // Add once registers for tick system registration.
              static #tick_system_ident_register: [std::sync::Once; 8] = [#(#repeated),*];

              // System
              #[moonwave_core::system]
              #[write_component(moonwave_core::Actor)]
              #component_streams
              fn #tick_system_ident(#[state] level: &usize, #[resource] elapsed: &moonwave_core::FrameElapsedTime, world: &mut legion::world::SubWorld) {
                use legion::IntoQuery;
                moonwave_core::optick::event!(#event);
                let mut query = <(legion::Entity, &mut moonwave_core::Actor, #query_types)>::query();
                for (entity, actor, #names) in query.iter_mut(world).filter(|(_, actor, ..)| actor.level == *level) {
                  #rw
                  #({ #ticks })*
                  #event_receiver_impl
                }
              }
            }),
            Some(quote! {
              #tick_system_ident_register [level].call_once(|| {
                moonwave_core::Core::get_instance()
                  .get_world()
                  .add_system_to_stage(
                    Box::new(move || -> Box<dyn legion::systems::ParallelRunnable> { Box::new(#tick_system_create_ident(level) ) } ),
                    moonwave_core::SystemStage::Application(level as u8)
                  );
              });
            }),
          )
        } else {
          (None, None)
        };

        let optick_spawn = format!("Actor::{}::spawn", ident.to_string());

        quote! {
          #spawn_system

          #tick_system

          impl #ident {
            #(#items)*
          }

          impl moonwave_core::Spawnable for #ident {
            fn spawn(
              self,
              parent: Option<moonwave_core::Entity>,
              level: usize,
              cmd: &mut legion::systems::CommandBuffer,
            ) -> moonwave_core::ActorRc<Self> {
              moonwave_core::optick::event!(#optick_spawn);

              // Lazily insert tick system.
              #tick_register

              // Create timers.
              let mut timers = Vec::new();
              #(#timer_setup)*

              // Spawn actor into world.
              #before_move
              let actor_ref = moonwave_core::ActorRc::new(cmd, self, parent, level, timers);
              let entity = actor_ref.get_entity();

              // Add event receiver
              #event_receiver_spawn

              // Temporarly insert spawn system.
              #spawn_system_register

              actor_ref
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

#[derive(Clone, Debug)]
struct ActorMethod {
  method: ImplItemMethod,
  self_usage: ComponentUsage,
  usages: Vec<ComponentUsage>,
}

#[derive(Clone, Debug)]
struct ComponentUsage {
  mutable: bool,
  reader: bool,
  writer: bool,
  name: syn::Ident,
  component: syn::Path,
}

impl ComponentUsage {
  pub fn query_type(&self) -> TokenStream2 {
    let ident = &self.component;
    if self.mutable {
      quote! { &mut #ident }
    } else {
      quote! { &#ident }
    }
  }

  pub fn name(&self) -> syn::Ident {
    if self.reader || self.writer {
      return self.name.clone();
    }

    format_ident!("{}", path_to_string_ident(&self.component).to_snake_case())
  }

  pub fn extend_stream(&self, stream: &mut TokenStream2) {
    let ident = &self.component;
    if self.mutable {
      stream.extend(quote! {
        #[write_component(#ident)]
      });
    } else {
      stream.extend(quote! {
        #[read_component(#ident)]
      });
    }
  }
}

fn get_ident_of_type(ty: &syn::Type) -> syn::Ident {
  match ty {
    syn::Type::Path(path) => path.path.get_ident().unwrap().clone(),
    _ => panic!("Not a ident type"),
  }
}

fn get_path_of_type(ty: &syn::Type) -> syn::Path {
  match ty {
    syn::Type::Path(path) => path.path.clone(),
    _ => panic!("Not a ident type"),
  }
}

fn get_ident_of_pat(pat: &syn::Pat) -> syn::Ident {
  match pat {
    syn::Pat::Ident(ident) => ident.ident.clone(),
    syn::Pat::Path(path) => path.path.get_ident().unwrap().clone(),
    _ => panic!("Not a named pattern {:?}", pat),
  }
}

fn path_to_string_ident(path: &Path) -> String {
  path
    .segments
    .iter()
    .map(|segment| {
      let postfix = match &segment.arguments {
        PathArguments::AngleBracketed(x) => match x.args.first().as_ref().unwrap() {
          GenericArgument::Type(Type::Path(path)) => {
            format!("_{}_", path_to_string_ident(&path.path))
          }
          _ => "".to_string(),
        },
        _ => "".to_string(),
      };
      format!("{}{}", segment.ident.to_string(), postfix)
    })
    .collect::<Vec<_>>()
    .join("_")
}

impl ActorMethod {
  fn from_path(path: &Path, pat: &syn::Pat, mutable: bool) -> ComponentUsage {
    let reader = path
      .segments
      .first()
      .map(|seg| seg.ident.to_string() == "Reader")
      .unwrap_or(false);

    let writer = path
      .segments
      .first()
      .map(|seg| seg.ident.to_string() == "Writer")
      .unwrap_or(false);

    let reader_component = if reader || writer {
      let p = path.segments.first().unwrap();
      match &p.arguments {
        PathArguments::AngleBracketed(angle) => {
          let arg = angle.args.first().unwrap();
          match arg {
            GenericArgument::Type(Type::Path(path)) => Some(path.path.clone()),
            _ => unimplemented!(),
          }
        }
        _ => unimplemented!(),
      }
    } else {
      None
    };

    ComponentUsage {
      reader,
      writer,
      name: get_ident_of_pat(pat),
      mutable: if writer { true } else { mutable },
      component: reader_component.unwrap_or_else(|| path.clone()),
    }
  }

  pub fn new(self_ident: syn::Ident, method: &ImplItemMethod) -> Self {
    // Build self usage.
    let method_self = match method
      .sig
      .inputs
      .iter()
      .find(|input| matches!(input, FnArg::Receiver(..)))
    {
      Some(FnArg::Receiver(rec)) => Some(rec.clone()),
      _ => None,
    };
    let self_usage = ComponentUsage {
      name: format_ident!("self"),
      reader: false,
      writer: false,
      mutable: method_self
        .map(|m| m.mutability.is_some())
        .unwrap_or_default(),
      component: parse_quote! { #self_ident },
    };

    // Iterate through all other dependencies.
    let usages = method
      .sig
      .inputs
      .iter()
      .skip(1)
      .filter_map(|arg| match arg {
        FnArg::Typed(typed) => match &*typed.ty {
          Type::Path(path) => Some(Self::from_path(&path.path, &typed.pat, false)),
          Type::Reference(ty_ref) => match &*ty_ref.elem {
            Type::Path(path) => Some(Self::from_path(
              &path.path,
              &typed.pat,
              ty_ref.mutability.is_some(),
            )),
            _ => None,
          },
          _ => None,
        },
        _ => None,
      })
      .collect::<Vec<_>>();

    Self {
      method: method.clone(),
      self_usage,
      usages,
    }
  }

  pub fn combined(
    inputs: &[ActorMethod],
  ) -> (TokenStream2, TokenStream2, TokenStream2, TokenStream2) {
    let self_mutable = inputs.iter().any(|t| t.self_usage.mutable);
    let mut components = inputs
      .iter()
      .flat_map(|t| t.usages.clone())
      .collect::<Vec<_>>();
    components.sort_unstable_by_key(|u| path_to_string_ident(&u.component));
    components.dedup_by_key(|u| u.component.clone());

    // Build stream for component refs
    let component_streams = {
      let mut stream = TokenStream2::new();
      ComponentUsage {
        reader: false,
        writer: false,
        mutable: self_mutable,
        name: format_ident!("self"),
        component: inputs[0].self_usage.component.clone(),
      }
      .extend_stream(&mut stream);
      for usage in &components {
        usage.extend_stream(&mut stream);
      }
      stream
    };

    // Build stream for the query
    let query_stream = {
      let all = [ComponentUsage {
        reader: false,
        mutable: self_mutable,
        writer: false,
        name: format_ident!("self"),
        component: inputs[0].self_usage.component.clone(),
      }];
      let usages = all
        .iter()
        .chain(components.iter().filter(|c| !c.reader && !c.writer))
        .map(|m| m.query_type());

      quote! {
        #(#usages),*
      }
    };

    // Build stream for the query
    let names = {
      let all = [ComponentUsage {
        reader: false,
        writer: false,
        mutable: self_mutable,
        name: format_ident!("self"),
        component: inputs[0].self_usage.component.clone(),
      }];
      let usages = all
        .iter()
        .chain(components.iter().filter(|c| !c.reader && !c.writer))
        .map(|m| m.name());

      quote! {
        #(#usages),*
      }
    };

    // Build reader/writer streams
    let reader_writer = {
      let streams = components.iter().filter(|c| c.reader || c.writer).map(|c| {
        let name = &c.name;
        let component = &c.component;
        let marker_name = if c.reader {
          format_ident!("{}", "Reader")
        } else {
          format_ident!("{}", "Writer")
        };

        quote! {
          let #name: #marker_name<#component> = #marker_name { _p:  std::marker::PhantomData };
        }
      });
      quote! {
        #(#streams)*
      }
    };

    (component_streams, query_stream, names, reader_writer)
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

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum SpawnType {
  Blocking,
  Background,
}
/*
impl Parse for SpawnType {
  fn parse(input: ParseStream) -> Result<Self> {}
}*/

#[proc_macro_attribute]
pub fn actor_tick(_attr: TokenStream, _item: TokenStream) -> TokenStream {
  TokenStream::new()
}

#[proc_macro_attribute]
pub fn actor_spawn(_attr: TokenStream, _item: TokenStream) -> TokenStream {
  TokenStream::new()
}

enum ServiceTraitItem {
  TraitDef(Box<ItemTrait>),
  TraitImpl(Box<ItemImpl>),
}

impl Parse for ServiceTraitItem {
  fn parse(input: ParseStream) -> Result<Self> {
    let lookahead = input.lookahead1();
    if lookahead.peek(Token![trait]) || lookahead.peek(Token![pub]) {
      input.parse().map(ServiceTraitItem::TraitDef)
    } else if lookahead.peek(Token![impl]) {
      input.parse().map(ServiceTraitItem::TraitImpl)
    } else {
      Err(lookahead.error())
    }
  }
}

fn service_trait_logger_items(
  ident: &proc_macro2::Ident,
  trait_items: &[TraitItem],
) -> Vec<TokenStream2> {
  trait_items
    .iter()
    .map(|item| match item {
      TraitItem::Method(method) => {
        let sig = method.sig.clone();
        let sig_args = method.sig.inputs.iter().filter_map(|input| match input {
          FnArg::Typed(ty) => Some(ty.pat.clone()),
          _ => None,
        });
        let name = method.sig.ident.clone();
        let name_str = name.to_string();
        let log_msg = format!("Service call >> {}::{}", ident.to_string(), name_str);

        quote! {
          #sig {
            moonwave_core::debug!(#log_msg);
            self.0.#name(#(#sig_args),*)
          }
        }
      }
      _ => quote! {},
    })
    .collect::<Vec<_>>()
}

fn service_trait_bench_items(
  ident: &proc_macro2::Ident,
  trait_items: &[TraitItem],
) -> Vec<TokenStream2> {
  trait_items
    .iter()
    .map(|item| match item {
      TraitItem::Method(method) => {
        let sig = method.sig.clone();
        let sig_args = method.sig.inputs.iter().filter_map(|input| match input {
          FnArg::Typed(ty) => Some(ty.pat.clone()),
          _ => None,
        });
        let name = method.sig.ident.clone();
        let name_str = name.to_string();
        let event_name = format!("Service::{}::{}", ident.to_string(), name_str);

        quote! {
          #sig {
            moonwave_core::optick::event!(#event_name);
            self.0.#name(#(#sig_args),*)
          }
        }
      }
      _ => quote! {},
    })
    .collect::<Vec<_>>()
}

fn generate_extension_tree(
  host: &proc_macro2::Ident,
  org: &proc_macro2::Ident,
  ext: &proc_macro2::Ident,
  items: &[TokenStream2],
) -> TokenStream2 {
  let ext_into = format_ident!("{}{}Into", ext, host);

  quote! {
        #[doc(hidden)]
        pub struct #ext (#host);
        impl #org for #ext {
          #(#items)*
        }
        impl moonwave_core::ServiceSafeType for #ext {}
        impl moonwave_core::TypedServiceIntoHost for #ext {
          type Host = #host;
          fn into_host(self) -> #host {
            #host {
              inner: std::sync::Arc::new(self),
            }
          }
        }
        pub trait #ext_into {
          fn #ext (self) -> #ext;
        }
        impl<T: moonwave_core::TypedServiceIntoHost<Host = #host>> #ext_into for T {
          fn #ext (self) -> #ext {
            #ext (self.into_host())
          }
        }
  }
}

#[proc_macro_attribute]
pub fn service_trait(_attr: TokenStream, item: TokenStream) -> TokenStream {
  let service = parse_macro_input!(item as ServiceTraitItem);
  match service {
    ServiceTraitItem::TraitDef(def) => {
      let name = def.ident.clone();
      let logged_items = service_trait_logger_items(&def.ident, &def.items);
      let benched_items = service_trait_bench_items(&def.ident, &def.items);

      let mut renamed = def;
      renamed.ident = format_ident!("{}ServiceTrait", renamed.ident.clone());
      let renamed_name = renamed.ident.clone();

      let logged_ext = generate_extension_tree(
        &name,
        &renamed.ident,
        &format_ident!("logged"),
        &logged_items,
      );
      let benched_ext = generate_extension_tree(
        &name,
        &renamed.ident,
        &format_ident!("benched"),
        &benched_items,
      );

      let items = renamed
        .items
        .iter()
        .map(|item| match item {
          TraitItem::Method(method) => {
            let sig = method.sig.clone();
            let sig_args = method.sig.inputs.iter().filter_map(|input| match input {
              FnArg::Typed(ty) => Some(ty.pat.clone()),
              _ => None,
            });
            let name = method.sig.ident.clone();

            quote! {
              pub #sig {
                self.inner.#name(#(#sig_args),*)
              }
            }
          }
          _ => quote! {},
        })
        .collect::<Vec<_>>();

      TokenStream::from(quote! {
        #renamed

        pub struct #name {
          inner: std::sync::Arc<dyn #renamed_name + Send + Sync + 'static>,
        }
        impl moonwave_core::ServiceSafeType for #name {}

        impl #name {
          #(#items)*
        }

        impl moonwave_core::TypedServiceTrait for #name {
          type Host = #name;
        }

        #logged_ext
        #benched_ext
      })
    }
    ServiceTraitItem::TraitImpl(mut imp) => {
      let (_, target_path, _) = imp.trait_.clone().unwrap();
      let service_trait_ident =
        format_ident!("{}ServiceTrait", target_path.get_ident().unwrap().clone());
      let new_target_path_stream = TokenStream::from(quote! { #service_trait_ident });
      let new_target_path = parse_macro_input!(new_target_path_stream as syn::Path);
      imp.trait_.as_mut().unwrap().1 = new_target_path;

      let host = target_path.get_ident().unwrap().clone();
      let selfness = if let Type::Path(p) = &*imp.self_ty {
        p.path.get_ident().unwrap().clone()
      } else {
        panic!("Path is not supported for service trait implementations, use `use` above.")
      };

      TokenStream::from(quote! {
        #imp

        impl moonwave_core::ServiceSafeType for #selfness {}

        impl moonwave_core::TypedServiceTrait for #selfness {
          type Host = #host;
        }

        impl moonwave_core::TypedServiceIntoHost for #selfness {
          type Host = #host;
          fn into_host(self) -> #host {
            #host {
              inner: std::sync::Arc::new(self),
            }
          }
        }
      })
    }
  }
}
