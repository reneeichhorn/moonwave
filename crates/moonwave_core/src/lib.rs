#![allow(clippy::new_without_default)]
#![feature(get_mut_unchecked)]

mod application;
mod base;
mod ecs;
mod execution;
mod extension;
mod glyph;
mod logger;
mod nodes;
mod service;
mod memory;

pub use application::*;
pub use base::{BindGroupLayoutSingleton, Core, OnceInFrame, ShaderKind, TaskKind};
pub use ecs::*;
pub use extension::*;
pub use glyph::*;
pub use logger::*;
pub use nodes::{PresentToScreen, TextureGeneratorHost, TextureGeneratorNode, TextureSize};
pub use service::*;
pub use memory::*;

pub use async_trait::async_trait;
pub use futures::{executor::block_on, Future};
pub use once_cell::sync::OnceCell;

pub use moonwave_core_macro::{actor, actor_spawn, actor_tick, service_trait};

pub use itertools::Itertools;
pub use optick;
pub use rayon;
