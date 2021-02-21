#![allow(clippy::new_without_default)]
#![feature(get_mut_unchecked)]

mod application;
mod base;
mod ecs;
mod execution;
mod extension;
mod logger;
mod nodes;

pub use application::*;
pub use base::{Core, TaskKind};
pub use ecs::*;
pub use execution::EstimatedExecutionTime;
pub use extension::*;
pub use logger::*;

pub use async_trait::async_trait;
pub use futures::{executor::block_on, Future};

pub use moonwave_core_macro::{actor, actor_spawn, actor_tick};
