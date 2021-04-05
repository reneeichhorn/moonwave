#![allow(clippy::new_without_default)]

use std::{cell::RefCell, rc::Rc};

mod layout;
mod layout_extension;
mod render;
mod stacks;
mod view;
pub use layout::*;
pub use layout_extension::*;
pub use render::*;
pub use stacks::*;
pub use view::*;

pub use moonwave_ui_macros::*;

pub trait Component {
  /// Get mutable reference to stored layout props of this component.
  fn get_layout_props_mut(&mut self) -> &mut LayoutProps;
  /// Get reference to stored layout props of this component.
  fn get_layout_props(&self) -> &LayoutProps;
  /// Offers the component a space and returns the actually needed space.
  fn offer_layout(&self, size: (f32, f32)) -> (f32, f32);

  /// Creates the component all its children the first time.
  fn create(&mut self, alloc: &mut Allocator) -> Option<ChildrenProxy>;
  /// Handles any partial update that has to happen to the component
  fn update(&mut self, updates: Box<dyn UpdateList>);

  /// Mounts and renders the actual component.
  fn mount(&mut self, size: (f32, f32), position: (f32, f32));
}

pub trait UpdateList {}

pub struct Allocator {}

impl Allocator {
  fn new() -> Self {
    Self {}
  }

  pub fn alloc<C: Component + 'static>(&mut self, component: C) -> HostedComponentRc {
    let mut boxed = Box::new(component);
    let children_proxy = boxed.create(self);

    Rc::new(RefCell::new(HostedComponent {
      component: boxed,
      children: Vec::new(),
      children_proxy,
    }))
  }
}

pub type HostedComponentRc = Rc<RefCell<HostedComponent>>;

pub struct HostedComponent {
  pub component: Box<dyn Component>,
  pub children: Vec<HostedComponentRc>,
  children_proxy: Option<ChildrenProxy>,
}

pub struct ChildrenProxy {
  component: HostedComponentRc,
}

impl ChildrenProxy {
  pub fn new(component: HostedComponentRc) -> Self {
    Self { component }
  }
}

impl HostedComponent {
  pub fn add_child(&mut self, child: HostedComponentRc) {
    if let Some(proxy) = &mut self.children_proxy {
      RefCell::borrow_mut(&proxy.component).add_child(child);
      return;
    }
    self.children.push(child);
  }
  pub fn insert_child(&mut self, index: usize, child: HostedComponentRc) {
    if let Some(proxy) = &mut self.children_proxy {
      RefCell::borrow_mut(&proxy.component).insert_child(index, child);
      return;
    }
    self.children.insert(index, child)
  }
}

pub struct AppRoot {
  layout: LayoutProps,
  proxy: Option<HostedComponentRc>,
}

impl AppRoot {
  pub fn new() -> Self {
    Self {
      layout: LayoutProps {
        frame: Some((500.0, 500.0)),
        ..Default::default()
      },
      proxy: None,
    }
  }
}

impl Component for AppRoot {
  fn get_layout_props(&self) -> &LayoutProps {
    &self.layout
  }
  fn get_layout_props_mut(&mut self) -> &mut LayoutProps {
    &mut self.layout
  }

  fn update(&mut self, updates: Box<dyn UpdateList>) {}

  fn create(&mut self, alloc: &mut Allocator) -> Option<ChildrenProxy> {
    let proxy = alloc.alloc(ChildrenCollectionProxy {});
    self.proxy = Some(proxy.clone());
    Some(ChildrenProxy { component: proxy })
  }

  fn offer_layout(&self, size: (f32, f32)) -> (f32, f32) {
    self.layout.frame.unwrap()
  }

  fn mount(&mut self, _size: (f32, f32), _position: (f32, f32)) {
    let proxy = RefCell::borrow_mut(self.proxy.as_ref().unwrap());
    if proxy.children.len() != 1 {
      panic!("AppRoot component must have exactly one child");
    }
    let mut child = RefCell::borrow_mut(&proxy.children[0]);
    let wanted = child.component.offer_layout(self.layout.frame.unwrap());
    child.component.mount(wanted, (0.0, 0.0));
  }
}

pub struct UIRenderer {
  allocator: Allocator,
  root: HostedComponentRc,
}

impl UIRenderer {
  pub fn new(component: impl Component + 'static) -> Self {
    let mut allocator = Allocator::new();
    let root = allocator.alloc(component);

    Self { root, allocator }
  }

  pub fn mount(&self) {
    // Layouting phase
    let mut root = RefCell::borrow_mut(&self.root);
    let root_layout = root.component.offer_layout((0.0, 0.0));

    // Mounting phase
    root.component.mount(root_layout, (0.0, 0.0));
  }
}

pub struct ChildrenCollectionProxy;
impl Component for ChildrenCollectionProxy {
  fn get_layout_props(&self) -> &LayoutProps {
    unimplemented!()
  }
  fn get_layout_props_mut(&mut self) -> &mut LayoutProps {
    unimplemented!()
  }
  fn create(&mut self, alloc: &mut Allocator) -> Option<ChildrenProxy> {
    None
  }
  fn update(&mut self, updates: Box<dyn UpdateList>) {
    unimplemented!()
  }
  fn mount(&mut self, size: (f32, f32), position: (f32, f32)) {
    unimplemented!()
  }
  fn offer_layout(&self, size: (f32, f32)) -> (f32, f32) {
    size
  }
}
