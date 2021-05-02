use crate::Core;
use async_std::sync::{RwLock as AsyncRwLock, RwLockWriteGuard as AsyncRwLockWriteGuard};
use futures::{executor::block_on, future::join_all, Future};
use itertools::Itertools;
use legion::*;
use legion::{query::EntityFilter, World as LegionWorld};
pub use legion::{system, Entity};
use log::debug;
use once_cell::sync::OnceCell;
use owning_ref::{OwningRef, OwningRefMut};
use parking_lot::{Mutex, RwLock};
use rayon::ThreadPool;
use send_wrapper::SendWrapper;
use std::{
  marker::PhantomData,
  pin::Pin,
  sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Weak,
  },
};

pub struct World {
  /// Reference to legions ecs world.
  pub(crate) world: LegionWorld,
  /// All system factories that are evaluated when a new system is added or an old is removed.
  systems: RwLock<Vec<(usize, Box<dyn SystemFactory>)>>,
  systems_dirty: AtomicBool,
  /// Built system schedulers for each stage.
  built_systems: RwLock<Vec<SendWrapper<Schedule>>>,
  /// Temporary systems that are always executed just once.
  temp_systems: Mutex<Vec<Box<dyn ParallelRunnable>>>,
  /// Temporary systems that are always executed just once.
  event_systems: Mutex<Vec<Box<dyn ParallelRunnable>>>,
  /// Command buffers that are waiting to be executed.
  command_buffers: Mutex<Vec<(CommandBuffer, Option<Arc<ActorInnerRef>>)>>,
}

impl World {
  /// Creates a new empty world without entities and systems.
  pub fn new() -> Self {
    let mut world = LegionWorld::new(WorldOptions::default());
    world.subscribe(EventLogger {}, legion::any());

    Self {
      systems_dirty: AtomicBool::new(false),
      built_systems: RwLock::new(Vec::new()),
      world,
      systems: RwLock::new(Vec::new()),
      event_systems: Mutex::new(Vec::with_capacity(128)),
      temp_systems: Mutex::new(Vec::with_capacity(128)),
      command_buffers: Mutex::new(Vec::with_capacity(128)),
    }
  }

  /// Adds a temporary system to the world that will be executed exactly once.
  pub fn add_temp_system(&self, system: Box<dyn ParallelRunnable>) {
    let mut staging = self.temp_systems.lock();
    staging.push(system);
  }

  /// Schedule event
  pub fn publish_event<T: Component + Clone + Sized + 'static>(&self, event: T) {
    let mut systems = self.event_systems.lock();
    systems.push(Box::new(actor_event_publish_system(event)));
  }

  /// Adds a temporary system to the world that will be executed exactly once.
  pub(crate) fn add_command_buffer(
    &self,
    cmd: CommandBuffer,
    front: bool,
    owner: Option<Arc<ActorInnerRef>>,
  ) {
    let mut staging = self.command_buffers.lock();
    if front {
      staging.insert(0, (cmd, owner));
    } else {
      staging.push((cmd, owner));
    }
  }

  /// Adds a system to the default application stage causing the system tree to be
  /// marked as dirty and therefore will trigger rebuilding in the background
  pub fn add_system<S: SystemFactory>(&self, system: S) {
    self.add_system_to_stage(system, SystemStage::Application(0));
  }

  /// Adds a system to a specific stage causing the system tree to be
  /// marked as dirty and therefore will trigger rebuilding in the background
  pub fn add_system_to_stage<S: SystemFactory>(&self, system: S, stage: SystemStage) {
    let mut systems = self.systems.write();
    systems.push((stage.order_num(), Box::new(system)));
    systems.sort_unstable_by_key(|(order_num, _)| *order_num);
    self.systems_dirty.store(true, Ordering::Relaxed);
  }

  /// Rebuilds the entire schedule tree for all systems in the background
  /// and replaces the active systems once done.
  fn rebuild_schedule(&self) {
    if !self.systems_dirty.load(Ordering::Acquire) {
      return;
    }

    optick::event!("World::rebuild_schedule");
    self.systems_dirty.store(false, Ordering::Relaxed);

    // Group by stage.
    let systems = self.systems.read();
    let groups = systems.iter().group_by(|(order_num, _)| *order_num);

    // Each groups creates a new schedule.
    let mut built = Vec::new();
    for (_, group) in &groups {
      let mut builder = Schedule::builder();
      for (_, system) in group {
        builder.add_system(system.create_system());
      }
      built.push(SendWrapper::new(builder.build()));
    }

    // Update actual system schedules
    *self.built_systems.write() = built;
  }

  pub(crate) fn execute_commands(&mut self, resources: &mut Resources) {
    optick::event!("World::tick::command_buffers");

    #[allow(clippy::needless_collect)]
    {
      let buffers = self.command_buffers.lock().drain(..).collect::<Vec<_>>();
      for (mut buffer, _owner) in buffers.into_iter() {
        buffer.flush(&mut self.world, resources);
      }
    }
  }

  pub fn tick(&mut self, elapsed: u64, pool: &ThreadPool) {
    // Trigger schedule rebuilding if needed.
    self.rebuild_schedule();

    // Execute all systems grouped by stage.
    let mut resources = Resources::default();
    resources.insert(FrameElapsedTime(elapsed));

    // Execute
    self.execute_commands(&mut resources);

    // Event systems
    {
      optick::event!("World::tick::event");
      loop {
        optick::event!("World::tick::event::iteration");

        // Drain event systems until empty.
        let systems = {
          let mut systems = self.event_systems.lock();
          systems.drain(..).collect::<Vec<_>>()
        };
        if systems.is_empty() {
          break;
        }

        // Execute systems
        let mut builder = Schedule::builder();
        for temp in systems {
          builder.add_system(WrappedSystem(temp));
        }

        {
          optick::event!("World::tick::event::iteration::execute");
          builder
            .build()
            .execute_in_thread_pool(&mut self.world, &mut resources, pool);
        }
      }
    }

    // Execute world systems
    {
      optick::event!("World::tick::systems");
      let mut systems = self.built_systems.write();
      for system in systems.iter_mut() {
        system.execute_in_thread_pool(&mut self.world, &mut resources, pool)
      }
    }

    // Execute temp systems
    {
      optick::event!("World::tick::temp_systems");
      let systems = self.temp_systems.lock().drain(..).collect::<Vec<_>>();
      let mut builder = Schedule::builder();
      for temp in systems {
        builder.add_system(WrappedSystem(temp));
      }
      builder
        .build()
        .execute_in_thread_pool(&mut self.world, &mut resources, pool)
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Res {
  a: f32,
}

/// System resource that holds the elapsed time since previous frame.
pub struct FrameElapsedTime(pub u64);

/// Simple timer handler that allows to execute specific content every x ms.
pub struct Timer {
  pub every_micros: u64,
  pub elapsed: u64,
  pub dirty: bool,
}

impl Timer {
  pub fn tick(&mut self, elapsed: u64) {
    self.elapsed += elapsed;

    if self.elapsed >= self.every_micros {
      self.dirty = true;
      self.elapsed = 0;
    }
  }
}

pub trait SystemFactory: Send + Sync + 'static {
  fn create_system(&self) -> WrappedSystem;
}

impl<F: Fn() -> Box<dyn ParallelRunnable> + Send + Sync + 'static> SystemFactory for F {
  fn create_system(&self) -> WrappedSystem {
    WrappedSystem(self())
  }
}

pub type TraitFuture = Pin<Box<dyn Future<Output = ()>>>;
pub type TraitFutureSendSync = Pin<Box<dyn Future<Output = ()> + Send + Sync>>;

pub trait WorldScheduler {
  fn get_thread_pool(&self) -> &ThreadPool;
  fn schedule(&self, task: TraitFuture) -> TraitFuture;
  fn schedule_bg(&self, task: TraitFuture) -> TraitFuture;
}

use legion::storage::*;
use legion::systems::*;
use legion::world::*;

pub struct WrappedSystem(pub Box<dyn ParallelRunnable>);
impl Runnable for WrappedSystem {
  fn accesses_archetypes(&self) -> &ArchetypeAccess {
    self.0.accesses_archetypes()
  }
  fn command_buffer_mut(&mut self, world: WorldId) -> Option<&mut CommandBuffer> {
    self.0.command_buffer_mut(world)
  }
  fn name(&self) -> Option<&SystemId> {
    self.0.name()
  }
  fn prepare(&mut self, world: &LegionWorld) {
    self.0.prepare(world)
  }
  fn reads(&self) -> (&[ResourceTypeId], &[ComponentTypeId]) {
    self.0.reads()
  }
  fn run(&mut self, world: &mut LegionWorld, resources: &mut Resources) {
    self.0.run(world, resources)
  }
  unsafe fn run_unsafe(&mut self, world: &LegionWorld, resources: &UnsafeResources) {
    self.0.run_unsafe(world, resources)
  }
  fn writes(&self) -> (&[ResourceTypeId], &[ComponentTypeId]) {
    self.0.writes()
  }
}

/// The system stage specifies the order the system is executed at.
pub enum SystemStage {
  /// The cold stage is for system that should run as soon as possible when the frame just started.
  Cold,
  /// Application level logic for any non-engine systems or system without order dependence.
  Application(u8),
  /// The rendering prep stage is used for system that are required right before the rendering onto the screen.
  RenderingPreperations,
  /// The rendering stage is executed after the application changed their actors / uniforms / buffers etc. and is ready to be rendered.
  Rendering,
}
impl SystemStage {
  pub fn order_num(&self) -> usize {
    match self {
      SystemStage::Cold => 0,
      SystemStage::Application(i) => *i as usize + 1,
      SystemStage::RenderingPreperations => u8::MAX as usize + 2,
      SystemStage::Rendering => u8::MAX as usize + 3,
    }
  }
}

/// Spawnable defines an data type that can be spawned into the ecs world.
pub trait Spawnable: Sized {
  fn spawn(self, parent: Option<Entity>, level: usize, cmd: &mut CommandBuffer) -> ActorRc<Self>;
}

pub struct UnlockedSpawn<'a, T: Spawnable> {
  entity: &'a Entity,
  spawn: &'a T,
  actor: &'a Actor,
  cmd: &'a mut CommandBuffer,
}

impl<'a, T: Spawnable> UnlockedSpawn<'a, T> {
  pub fn new(
    actor: &'a Actor,
    entity: &'a Entity,
    spawn: &'a T,
    cmd: &'a mut CommandBuffer,
  ) -> Self {
    Self {
      entity,
      spawn,
      cmd,
      actor,
    }
  }

  pub fn spawn_actor<S: Spawnable>(&mut self, s: S) -> ActorRc<S> {
    s.spawn(Some(*self.entity), self.actor.level + 1, self.cmd)
  }

  pub fn add_component<C: Component>(&mut self, c: C) {
    self.cmd.add_component(*self.entity, c);
  }

  pub fn remove_component<C: Component>(&mut self) {
    self.cmd.remove_component::<C>(*self.entity);
  }
}

impl<'a, T: Spawnable> std::ops::Deref for UnlockedSpawn<'a, T> {
  type Target = T;
  fn deref(&self) -> &Self::Target {
    self.spawn
  }
}

pub struct UnlockedSpawnMut<'a, T: Spawnable> {
  entity: &'a Entity,
  spawn: &'a mut T,
  actor: &'a Actor,
  cmd: &'a mut CommandBuffer,
}

impl<'a, T: Spawnable> UnlockedSpawnMut<'a, T> {
  pub fn new(
    actor: &'a Actor,
    entity: &'a Entity,
    spawn: &'a mut T,
    cmd: &'a mut CommandBuffer,
  ) -> Self {
    Self {
      entity,
      spawn,
      cmd,
      actor,
    }
  }

  pub fn spawn_actor<S: Spawnable>(&mut self, s: S) -> ActorRc<S> {
    s.spawn(Some(*self.entity), self.actor.level + 1, self.cmd)
  }

  pub fn add_component<C: Component>(&mut self, c: C) {
    self.cmd.add_component(*self.entity, c);
  }

  pub fn remove_component<C: Component>(&mut self) {
    self.cmd.remove_component::<C>(*self.entity);
  }
}

impl<'a, T: Spawnable> std::ops::Deref for UnlockedSpawnMut<'a, T> {
  type Target = T;
  fn deref(&self) -> &Self::Target {
    self.spawn
  }
}

impl<'a, T: Spawnable> std::ops::DerefMut for UnlockedSpawnMut<'a, T> {
  fn deref_mut(&mut self) -> &mut Self::Target {
    self.spawn
  }
}

pub struct WeakSpawn<T: Spawnable> {
  actor: Arc<ActorInnerRef>,
  cmd: CommandBuffer,
  _p: PhantomData<T>,
}

pub struct SubWeakSpawn<'a, T: Spawnable> {
  cmd: &'a mut CommandBuffer,
  entity: Entity,
  _level: usize,
  rc: ActorRc<T>,
}

impl<'a, T: Spawnable> SubWeakSpawn<'a, T> {
  pub fn add_component<C: Component>(&mut self, c: C) {
    self.cmd.add_component(self.entity, c);
  }

  pub fn remove_component<C: Component>(&mut self) {
    self.cmd.remove_component::<C>(self.entity);
  }

  pub fn into_rc(self) -> ActorRc<T> {
    self.rc
  }
}

impl<T: Spawnable + Send + Sync + 'static> WeakSpawn<T> {
  fn new(inner: Arc<ActorInnerRef>) -> Self {
    Self {
      actor: inner,
      cmd: CommandBuffer::new(&Core::get_instance().get_world().world),
      _p: PhantomData {},
    }
  }

  pub fn spawn_actor<S: Spawnable + Send + Sync + 'static>(&mut self, s: S) -> SubWeakSpawn<S> {
    let actor = s.spawn(Some(self.actor.entity), self.actor.level + 1, &mut self.cmd);
    SubWeakSpawn {
      cmd: &mut self.cmd,
      entity: actor.inner.entity,
      rc: actor,
      _level: self.actor.level + 1,
    }
  }

  pub fn add_component<C: Component>(&mut self, c: C) {
    self.cmd.add_component(self.actor.entity, c);
  }

  pub fn remove_component<C: Component>(&mut self) {
    self.cmd.remove_component::<C>(self.actor.entity);
  }

  pub fn exec_mut<F: 'static + FnOnce(&mut UnlockedWeakSpawn<'_, T>) + Send + Sync>(
    &mut self,
    f: F,
  ) {
    let entity = self.actor.entity;
    let once = Mutex::new(Some(f));

    self.cmd.exec_mut(move |world, _| {
      let entry = world.entry(entity).unwrap();
      let mut unlocked = UnlockedWeakSpawn {
        entry,
        _p: PhantomData {},
      };
      let mut once_unlocked = once.lock();
      if let Some(f) = once_unlocked.take() {
        f(&mut unlocked);
      }
    });
  }

  pub fn flush(self) {
    Core::get_instance()
      .get_world()
      .add_command_buffer(self.cmd, false, Some(self.actor.clone()));
  }
}

pub struct UnlockedWeakSpawn<'a, T: Spawnable> {
  entry: Entry<'a>,
  _p: PhantomData<T>,
}

impl<'a, T: Spawnable> UnlockedWeakSpawn<'a, T> {
  pub fn get_component<C: Component>(&self) -> Result<&C, ComponentError> {
    self.entry.get_component::<C>()
  }

  pub fn get_component_mut<C: Component>(&mut self) -> Result<&mut C, ComponentError> {
    self.entry.get_component_mut::<C>()
  }

  pub fn add_component<C: Component>(&mut self, c: C) {
    self.entry.add_component(c)
  }

  pub fn remove_component<C: Component>(&mut self) {
    self.entry.remove_component::<C>()
  }
}

impl<'a, T: Spawnable + Send + Sync + 'static> std::ops::Deref for UnlockedWeakSpawn<'a, T> {
  type Target = T;
  fn deref(&self) -> &Self::Target {
    self.entry.get_component::<T>().unwrap()
  }
}

impl<'a, T: Spawnable + Send + Sync + 'static> std::ops::DerefMut for UnlockedWeakSpawn<'a, T> {
  fn deref_mut(&mut self) -> &mut Self::Target {
    self.entry.get_component_mut::<T>().unwrap()
  }
}

/// Base actor component added to _all_ spawned actors during runtime.
pub struct Actor {
  weak: Weak<ActorInnerRef>,
  /// The entity id of the parent of the spawned actor if available.
  pub parent: Option<Entity>,
  /// The level of the spawned actor.
  pub level: usize,
  #[doc(hidden)]
  pub timers: Vec<Timer>,
}

impl Actor {
  pub fn get_weak<T: Spawnable + Send + Sync + 'static>(&self) -> Option<WeakSpawn<T>> {
    self.weak.upgrade().map(WeakSpawn::new)
  }
}

// Actor reference collection and actor despawning on drop.
// -----------------------------------------------------------------------------

pub struct ActorRc<T> {
  inner: Arc<ActorInnerRef>,
  _p: PhantomData<T>,
}

impl<T> Clone for ActorRc<T> {
  fn clone(&self) -> Self {
    Self {
      inner: self.inner.clone(),
      _p: PhantomData {},
    }
  }
}

impl<T: Send + Sync + 'static> ActorRc<T> {
  pub fn new(
    cmd: &mut CommandBuffer,
    value: T,
    parent: Option<Entity>,
    level: usize,
    timers: Vec<Timer>,
  ) -> Self {
    let entity = cmd.push((value, Res { a: 0.0 }));
    let arc = Arc::new(ActorInnerRef { entity, level });
    cmd.add_component(
      entity,
      Actor {
        level,
        parent,
        timers,
        weak: Arc::downgrade(&arc),
      },
    );

    ActorRc {
      inner: arc,
      _p: PhantomData {},
    }
  }

  pub fn get_entity(&self) -> Entity {
    self.inner.entity
  }

  pub fn entry(&self) -> ActorEntry<'_> {
    let entry = Core::get_instance()
      .get_world()
      .world
      .entry_ref(self.inner.entity);

    // Unwrap is safe here as entity will be only dropped when all reference to it are removed
    ActorEntry {
      entry: entry.unwrap(),
    }
  }
}

impl<T: Spawnable + Send + Sync + 'static> ActorRc<T> {
  pub fn get_weak(&self) -> WeakSpawn<T> {
    WeakSpawn::new(self.inner.clone())
  }
}

pub(crate) struct ActorInnerRef {
  entity: Entity,
  level: usize,
}
pub struct WrappedEntity(pub Entity);

impl Drop for ActorInnerRef {
  fn drop(&mut self) {
    let world = Core::get_instance().get_world();
    world.add_temp_system(Box::new(actor_drop_system_system(WrappedEntity(
      self.entity,
    ))));
  }
}

#[system]
fn actor_drop_system(#[state] entity: &WrappedEntity, cmd: &mut CommandBuffer) {
  cmd.remove(entity.0);
}

pub struct Reader<T: Send + Sync + 'static> {
  pub _p: PhantomData<T>,
}
impl<T: Send + Sync + 'static> Clone for Reader<T> {
  fn clone(&self) -> Reader<T> {
    Reader { _p: PhantomData }
  }
}
impl<T: Send + Sync + 'static> Copy for Reader<T> {}

pub struct ActorEntry<'a> {
  pub entry: EntryRef<'a>,
}

impl<'a> ActorEntry<'a> {
  pub fn get<C: Send + Sync + 'static>(&self, _reader: Reader<C>) -> Option<&C> {
    self.entry.get_component::<C>().ok()
  }
}

// Event system
/////////////////////////////////////////////////////////

pub struct EventReceiver<T: Component + Clone + Sized + 'static> {
  received: Vec<T>,
}

impl<T: Component + Clone + Sized + 'static> EventReceiver<T> {
  pub fn new() -> Self {
    Self {
      received: Vec::new(),
    }
  }
  pub fn drain(&mut self) -> std::vec::Drain<T> {
    self.received.drain(..)
  }
}

#[system(for_each)]
fn actor_event_publish<T: Component + Clone + Sized + 'static>(
  receiver: &mut EventReceiver<T>,
  #[state] event: &T,
) {
  receiver.received.push(event.clone());
}

struct EventLogger;
impl EventSender for EventLogger {
  fn send(&self, event: Event) -> bool {
    //debug!("Legion event received | {:?}", event);
    true
  }
}
