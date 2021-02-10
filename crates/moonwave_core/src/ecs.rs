use crate::{Core, TaskKind};
use hecs::World as HecsWorld;
pub use hecs::{DynamicBundle, Entity, EntityBuilder, Query, QueryBorrow};
use std::sync::{Arc, RwLock};

pub struct World {
  world: HecsWorld,
  systems: RwLock<Vec<Arc<RwLock<dyn System>>>>,
  scheduled_spawns: RwLock<Vec<(Entity, EntityBuilder)>>,
}

pub trait System: Send + Sync {
  fn execute_system(&mut self, _core: Arc<Core>, elapsed: u64);
}

impl World {
  pub fn new() -> Self {
    Self {
      world: HecsWorld::new(),
      systems: RwLock::new(Vec::new()),
      scheduled_spawns: RwLock::new(Vec::new()),
    }
  }

  pub fn query<Q: Query>(&self) -> QueryBorrow<'_, Q> {
    self.world.query::<Q>()
  }

  pub fn add_system<T: System + 'static>(&self, system: T) {
    let mut systems = self.systems.write().unwrap();
    systems.push(Arc::new(RwLock::new(system)));
  }

  pub fn reserve(&self) -> Entity {
    self.world.reserve_entity()
  }

  pub fn spawn_at(&self, entity: Entity, builder: EntityBuilder) {
    // Store so it can be spawned later on.
    let mut spawns = self.scheduled_spawns.write().unwrap();
    spawns.push((entity, builder));
  }

  pub(crate) fn execute(&self, core: Arc<Core>, elapsed: u64) {
    let systems = self.systems.read().unwrap();
    for system in systems.iter().cloned() {
      let cloned_core = core.clone();
      let _ = core.schedule_task(TaskKind::ECS, async move {
        let mut sys = system.write().unwrap();
        sys.execute_system(cloned_core, elapsed);
      });
    }
  }

  pub(crate) fn handle_mutations(&mut self) {
    let mut spawns = self.scheduled_spawns.write().unwrap();
    for (entity, builder) in spawns.iter_mut() {
      self.world.spawn_at(*entity, builder.build());
    }
    spawns.clear();
  }
}

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

pub struct ActorBaseExt {
  pub entity: Entity,
  pub core: Arc<Core>,
  pub timers: Vec<Timer>,
}

impl ActorBaseExt {
  pub fn new(core: Arc<Core>) -> Self {
    Self {
      core,
      entity: Entity::from_bits(0),
      timers: Vec::new(),
    }
  }
  pub fn tick(&mut self, elapsed: u64) {
    for timer in &mut self.timers {
      timer.tick(elapsed);
    }
  }
}

pub trait IntoActor<T: Actor> {
  fn into_actor(self, _core: Arc<Core>) -> T;
}

pub trait Actor {
  fn get_core(&self) -> &Arc<Core> {
    &self.get_actor_ext().core
  }
  fn get_actor_ext(&self) -> &ActorBaseExt;
  fn get_actor_ext_mut(&mut self) -> &mut ActorBaseExt;
  fn into_raw_entity(self) -> Entity;
}
