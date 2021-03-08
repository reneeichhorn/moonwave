use std::{any::Any, sync::Arc};
use std::{any::TypeId, collections::HashMap};

use parking_lot::RwLock;

pub trait ServiceSafeType: Any + Send + Sync + 'static {}

pub trait TypedServiceTrait: 'static {
  type Host: ServiceSafeType;
}

pub trait TypedServiceIntoHost: 'static {
  type Host: ServiceSafeType;
  fn into_host(self) -> Self::Host;
}

pub struct ServiceLocator {
  systems: RwLock<HashMap<TypeId, Arc<dyn Any + Send + Sync + 'static>>>,
}

impl ServiceLocator {
  pub fn new() -> Self {
    Self {
      systems: RwLock::new(HashMap::new()),
    }
  }
  pub fn register<T: TypedServiceIntoHost>(&self, system: T) {
    let mut systems = self.systems.write();
    systems.insert(TypeId::of::<T::Host>(), Arc::new(system.into_host()));
  }

  pub fn discover<T: TypedServiceTrait>(&self) -> Arc<T::Host> {
    let systems = self.systems.read();
    let any = systems
      .get(&TypeId::of::<T::Host>())
      .unwrap_or_else(|| panic!("Tried to discover unknown service"));

    any
      .clone()
      .downcast::<T::Host>()
      .ok()
      .unwrap_or_else(|| panic!("Discovery of invalid type"))
  }
}
