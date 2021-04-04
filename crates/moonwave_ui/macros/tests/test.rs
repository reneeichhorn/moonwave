use moonwave_ui::*;
use moonwave_ui_macros::*;

struct MyComponent {
  foo: u32,
  storage: Storage,
  layout: LayoutProps,
}

#[component]
impl MyComponent {
  pub fn new() -> Self {
    Self {
      foo: 1,
      storage: Default::default(),
      layout: Default::default(),
    }
  }

  pub fn render(&self) {
    let foo = self.foo;

    render! {
      AppRoot {
        HStack {
          Foo(foo),
          Foo(foo),
          Foo(foo),
          ..children,
        }
      }
    }
  }
}

struct Foo {
  layout: LayoutProps,
}
impl Foo {
  fn new(foo: u32) -> Self {
    Self {
      layout: Default::default(),
    }
  }
}
impl moonwave_ui::Component for Foo {
  fn get_layout_props(&self) -> &LayoutProps {
    &self.layout
  }
  fn get_layout_props_mut(&mut self) -> &mut LayoutProps {
    &mut self.layout
  }
  fn offer_layout(&self, size: (f32, f32)) -> (f32, f32) {
    size
  }
  fn mount(&mut self, size: (f32, f32), position: (f32, f32)) {
    println!(
      "mounting foo @ {}x{} in {}x{}",
      size.0, size.1, position.0, position.1
    );
  }
  fn create(&mut self, alloc: &mut moonwave_ui::Allocator) -> Option<ChildrenProxy> {
    println!("creating foo");
    None
  }

  fn update(&mut self, updates: Box<dyn moonwave_ui::UpdateList>) {}
}

#[test]
fn test() {
  let renderer = UIRenderer::new(MyComponent::new());
  renderer.mount();
}
