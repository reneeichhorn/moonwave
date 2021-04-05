use std::cell::RefCell;

use crate::{
  Allocator, ChildrenCollectionProxy, ChildrenProxy, Component, HostedComponentRc, LayoutProps,
  UpdateList,
};

pub struct HStack {
  layout_props: LayoutProps,
  proxy: Option<HostedComponentRc>,
}

impl HStack {
  pub fn new() -> Self {
    Self {
      layout_props: Default::default(),
      proxy: None,
    }
  }
}

impl Component for HStack {
  fn get_layout_props(&self) -> &LayoutProps {
    &self.layout_props
  }
  fn get_layout_props_mut(&mut self) -> &mut LayoutProps {
    &mut self.layout_props
  }

  fn create(&mut self, alloc: &mut Allocator) -> Option<ChildrenProxy> {
    let proxy = alloc.alloc(ChildrenCollectionProxy {});
    self.proxy = Some(proxy.clone());
    Some(ChildrenProxy { component: proxy })
  }

  fn update(&mut self, updates: Box<dyn UpdateList>) {}

  fn mount(&mut self, size: (f32, f32), position: (f32, f32)) {
    let proxy = RefCell::borrow(self.proxy.as_ref().unwrap());
    let mut remaining_space = size.0;
    let mut remaining_children = proxy.children.len();

    // Measure sizes.
    let spaces = proxy.children.iter().map(|child| {
      let child = RefCell::borrow_mut(child);
      let offered = (
        remaining_space / remaining_children as f32 - self.layout_props.spacing.0 * 2.0,
        size.1 - self.layout_props.spacing.1 * 2.0,
      );
      let needed = child.component.offer_layout(offered);
      remaining_space -= needed.0;
      remaining_children -= 1;
      needed
    });

    // Mount
    let mut current_x = position.0;
    for (child, size) in proxy.children.iter().zip(spaces) {
      let mut child = RefCell::borrow_mut(child);
      child.component.mount(
        size,
        (
          current_x + self.layout_props.spacing.0,
          self.layout_props.spacing.1,
        ),
      );
      current_x += size.0 + self.layout_props.spacing.0;
    }
  }

  fn offer_layout(&self, size: (f32, f32)) -> (f32, f32) {
    size
  }
}
