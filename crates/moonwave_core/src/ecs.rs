use crate::Core;
use async_std::sync::{RwLock as AsyncRwLock, RwLockWriteGuard as AsyncRwLockWriteGuard};
use async_trait::async_trait;
use futures::{executor::block_on, future::join_all, Future};
use itertools::Itertools;
pub use legion::Entity;
use legion::{
  storage::{Component, ComponentTypeId, PackedStorage},
  systems::{CommandBuffer, ParallelRunnable, ResourceTypeId, Runnable, SystemId, UnsafeResources},
  world::{ArchetypeAccess, Entry, EntryMut, WorldId},
  EntityStore, Resources, Schedule, World as LegionWorld, WorldOptions,
};
use owning_ref::{OwningRef, OwningRefMut};
use parking_lot::RwLock;
use rayon::ThreadPool;
use send_wrapper::SendWrapper;
use std::{
  marker::PhantomData,
  pin::Pin,
  sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
  },
};

pub struct World {
  core: Arc<Core>,
  world: AsyncRwLock<LegionWorld>,
  systems: RwLock<Vec<(usize, Box<dyn SystemFactory>)>>,
  built_systems: RwLock<Vec<SendWrapper<Schedule>>>,
  systems_dirty: AtomicBool,
  root_actor: Entity,
}

impl World {
  /// Creates a new empty world without entities and systems.
  pub fn new<T: GenericIntoActor>(core: Arc<Core>, root: T) -> Self {
    // Create world with root actor.
    let mut world = LegionWorld::new(WorldOptions::default());

    let entity = world.push(());
    let actor = root.into_actor(core.clone(), entity);
    world.push_with_id(
      entity,
      (
        actor,
        ParentComponent {
          parent: None,
          children: Vec::new(),
        },
      ),
    );

    Self {
      core,
      systems_dirty: AtomicBool::new(false),
      built_systems: RwLock::new(Vec::new()),
      world: AsyncRwLock::new(world),
      systems: RwLock::new(Vec::new()),
      root_actor: entity,
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
      self.systems_dirty.store(false, Ordering::Relaxed);
      return;
    }

    optick::event!("World::rebuild_schedule");

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

  pub async fn quick_reserve(&self) -> Entity {
    let mut world = self.world.write().await;
    world.push(())
  }

  pub async fn spawn_actor<T: GenericIntoActor>(&self, actor: T, parent: Entity) -> ActorRef<T> {
    optick::event!("World::spawn_actor");

    let mut world = self.world.write().await;

    // Create new entity.
    let entity = actor
      .get_reserved_entity()
      .unwrap_or_else(|| world.push(()));
    let mut initial_children = Vec::new();
    actor.get_children(&mut initial_children);
    let actor = actor.into_actor(self.core.clone(), entity);
    world.push_with_id(
      entity,
      (
        actor,
        ParentComponent {
          parent: Some(parent),
          children: initial_children,
        },
        TickComponent {
          tick_fn: Arc::new(|core, entity, elapsed| {
            Box::pin(async move {
              optick::event!("World::actor::tick()");
              let mut world = core.get_world().world.write().await;
              let mut entry = world.entry_mut(entity).unwrap();

              // Tick actor itself.
              {
                let actor_comp = entry.get_component_mut::<T::Target>().unwrap();
                actor_comp.tick(elapsed).await;
              }

              // Tick all children.
              let children = entry
                .get_component::<ParentComponent>()
                .unwrap()
                .children
                .clone();

              drop(entry);

              let futures = children
                .into_iter()
                .map(|child| {
                  let child_entry = world.entry(child).unwrap();
                  (
                    child,
                    child_entry
                      .get_component::<TickComponent>()
                      .unwrap()
                      .tick_fn
                      .clone(),
                  )
                })
                .map(|(child, tick)| tick(core.clone(), child, elapsed));
              join_all(futures).await;
            })
          }),
        },
      ),
    );

    // Store reference in parent.
    let mut parent_entry = world.entry(parent).unwrap();
    let parent = parent_entry.get_component_mut::<ParentComponent>().unwrap();
    parent.children.push(entity);

    ActorRef {
      entity,
      _m: PhantomData::<T> {},
    }
  }

  fn despawn_actor_children_sync(
    guard: &mut AsyncRwLockWriteGuard<'_, LegionWorld>,
    entity: Entity,
  ) {
    // Remove  children
    let children = {
      let entry = guard.entry(entity).unwrap();
      let parent = entry.get_component::<ParentComponent>().unwrap();
      parent.children.clone()
    };
    for child in children {
      Self::despawn_actor_children_sync(guard, child);
    }

    // Remove actual actor entity itself.
    guard.remove(entity);
  }

  /// Removes an actor from the world.
  pub async fn despawn_actor(&self, entity: Entity) {
    optick::event!("World::despawn_actor");
    let mut world = self.world.write().await;

    // Update parent and remove reference.
    {
      let parent = {
        let parent_entry = world.entry(entity).unwrap();
        parent_entry
          .get_component::<ParentComponent>()
          .unwrap()
          .parent
      };
      if let Some(parent) = parent {
        let mut parent_entry = world.entry_mut(parent).unwrap();
        let component = parent_entry.get_component_mut::<ParentComponent>().unwrap();
        component.children.retain(|e| e != &entity);
      }
    }

    // Remove itself and all its children.
    Self::despawn_actor_children_sync(&mut world, entity);
  }

  pub fn tick(&self, elapsed: u64) {
    // Trigger schedule rebuilding if needed.
    self.rebuild_schedule();

    // Wait until world is unblocked, in this particular case this
    // should never be blocked as this is the beginning of the frame.
    let mut world = block_on(self.world.write());

    // Trigger root actor.
    {
      optick::event!("World::tick::root");
      let root = world.entry(self.root_actor).unwrap();
      if let Ok(tick) = root.get_component::<TickComponent>() {
        block_on((tick.tick_fn)(self.core.clone(), self.root_actor, elapsed));
      }
    }

    // Execute all systems grouped by stage.
    {
      optick::event!("World::tick::systems");
      let mut systems = self.built_systems.write();
      let mut resources = Resources::default();
      let pool = &self.core.execution.pool;
      for system in systems.iter_mut() {
        system.execute_in_thread_pool(&mut (*world), &mut resources, pool)
      }
    }
  }
}

/// Simple timer handler that allows to execute specific content every x ms.
pub struct Timer {
  pub every_ms: u64,
  pub elapsed: u64,
  pub dirty: bool,
}

impl Timer {
  pub fn tick(&mut self, elapsed: u64) {
    self.elapsed += elapsed;

    if self.elapsed >= self.every_ms {
      self.dirty = true;
      self.elapsed = 0;
    }
  }
}

/// The automatically injected base extension for each declared actor
/// containing all necessary references to its owner.
pub struct ActorBaseExt {
  pub entity: Entity,
  pub core: Arc<Core>,
  pub timers: Vec<Timer>,
}

impl ActorBaseExt {
  pub fn new(core: Arc<Core>, entity: Entity) -> Self {
    Self {
      core,
      entity,
      timers: Vec::new(),
    }
  }

  pub fn tick(&mut self, elapsed: u64) {
    for timer in &mut self.timers {
      timer.tick(elapsed);
    }
  }

  pub async fn spawn<T: GenericIntoActor>(&mut self, actor: T) -> ActorRef<T> {
    self.core.get_world().spawn_actor(actor, self.entity).await
  }

  pub async fn despawn(&self) {
    self.core.get_world().despawn_actor(self.entity).await
  }
}

pub struct TickComponent {
  tick_fn: Arc<dyn Fn(Arc<Core>, Entity, u64) -> TraitFuture + Send + Sync + 'static>,
}

pub struct ParentComponent {
  parent: Option<Entity>,
  children: Vec<Entity>,
}

/// Defines an object that can be made into a specific spawned actor.
pub trait GenericIntoActor: Send + Sync + 'static {
  type Target: Actor + Send + Sync + 'static;

  fn get_reserved_entity(&self) -> Option<Entity> {
    None
  }
  fn get_children(&self, _out: &mut Vec<Entity>) {}
  fn into_actor(self, _core: Arc<Core>, _entity: Entity) -> Self::Target;
}
#[async_trait]
pub trait Actor: Component<Storage = PackedStorage<Self>> + Send + Sync {
  fn get_core(&self) -> &Arc<Core> {
    &self.get_ext().core
  }

  fn get_ext(&self) -> &ActorBaseExt;

  fn get_ext_mut(&mut self) -> &mut ActorBaseExt;

  async fn tick(&mut self, elapsed: u64);
}

pub struct ParentActor<T: GenericIntoActor> {
  core: Arc<Core>,
  children: Vec<Entity>,
  reserved_entity: Entity,
  actor: Option<T>,
}

impl<T: GenericIntoActor> ParentActor<T> {
  pub async fn new(core: Arc<Core>) -> Self {
    let reserved_entity = core.get_world().quick_reserve().await;
    Self {
      core,
      reserved_entity,
      actor: None,
      children: Vec::new(),
    }
  }

  pub fn update(&mut self, actor: T) {
    self.actor = Some(actor);
  }

  pub async fn spawn_actor<A: GenericIntoActor>(&mut self, actor: A) -> ActorRef<A> {
    self
      .core
      .get_world()
      .spawn_actor(actor, self.reserved_entity)
      .await
  }
}

impl<T: GenericIntoActor> GenericIntoActor for ParentActor<T> {
  type Target = T::Target;

  fn get_children(&self, out: &mut Vec<Entity>) {
    out.extend(self.children.clone());
  }

  fn into_actor(self, core: Arc<Core>, entity: Entity) -> T::Target {
    self.actor.unwrap().into_actor(core, entity)
  }
}

/// A typed actor reference to an spawned entity.
pub struct ActorRef<T: GenericIntoActor> {
  pub(crate) entity: Entity,
  pub(crate) _m: PhantomData<T>,
}

impl<T: GenericIntoActor> ActorRef<T> {
  #[allow(clippy::needless_lifetimes)]
  pub async fn read<'a>(&self, core: &'a Core) -> ActorRefGuard<'a, T::Target> {
    let world = core.get_world().world.write().await;

    ActorRefGuard {
      inner_world_lock: world,
      entity: self.entity,
      _m: PhantomData::<T::Target> {},
    }
  }

  #[allow(clippy::needless_lifetimes)]
  pub async fn write<'a>(&self, core: &'a Core) -> ActorMutRefGuard<'a, T::Target> {
    let world = core.get_world().world.write().await;

    ActorMutRefGuard {
      inner_world_lock: world,
      entity: self.entity,
      _m: PhantomData::<T::Target> {},
    }
  }
}

pub struct ActorMutRefGuard<'a, T> {
  inner_world_lock: AsyncRwLockWriteGuard<'a, LegionWorld>,
  entity: Entity,
  _m: PhantomData<T>,
}

impl<'a, T: Send + Sync + 'static> ActorMutRefGuard<'a, T> {
  pub fn actor_mut(&mut self) -> OwningRefMut<Box<EntryMut<'_>>, T> {
    self.component_mut::<T>()
  }

  pub fn add_component<C: Send + Sync + 'static>(&mut self, component: C) {
    let mut entry = self.inner_world_lock.entry(self.entity).unwrap();
    entry.add_component(component);
  }

  pub fn remove_component<C: Send + Sync + 'static>(&mut self) {
    let mut entry = self.inner_world_lock.entry(self.entity).unwrap();
    entry.remove_component::<C>();
  }

  pub fn component_mut<C: Send + Sync + 'static>(&mut self) -> OwningRefMut<Box<EntryMut<'_>>, C> {
    let entry = self.inner_world_lock.entry_mut(self.entity).unwrap();
    OwningRefMut::new(Box::new(entry)).map_mut(|entry| entry.get_component_mut::<C>().unwrap())
  }
}

pub struct ActorRefGuard<'a, T> {
  inner_world_lock: AsyncRwLockWriteGuard<'a, LegionWorld>,
  entity: Entity,
  _m: PhantomData<T>,
}

impl<'a, T: Send + Sync + 'static> ActorRefGuard<'a, T> {
  pub fn actor(&'a mut self) -> OwningRef<Box<Entry<'a>>, T> {
    self.component::<T>()
  }

  pub fn component<C: Send + Sync + 'static>(&'a mut self) -> OwningRef<Box<Entry<'a>>, C> {
    let entry = self.inner_world_lock.entry(self.entity).unwrap();
    OwningRef::new(Box::new(entry)).map(|entry| entry.get_component::<C>().unwrap())
  }
}

pub trait SystemFactory: Send + Sync + 'static {
  fn create_system(&self) -> WrappedSystem;
}

pub type TraitFuture = Pin<Box<dyn Future<Output = ()>>>;
pub type TraitFutureSendSync = Pin<Box<dyn Future<Output = ()> + Send + Sync>>;

pub trait WorldScheduler {
  fn get_thread_pool(&self) -> &ThreadPool;
  fn schedule(&self, task: TraitFuture) -> TraitFuture;
  fn schedule_bg(&self, task: TraitFuture) -> TraitFuture;
}

pub struct WrappedSystem(Box<dyn ParallelRunnable>);
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
  /// The rendering stage is executed after the application changed their actors / uniforms / buffers etc. and is ready to be rendered.
  Rendering,
}
impl SystemStage {
  pub fn order_num(&self) -> usize {
    match self {
      SystemStage::Cold => 0,
      SystemStage::Application(i) => *i as usize + 1,
      SystemStage::Rendering => u8::MAX as usize + 2,
    }
  }
}
