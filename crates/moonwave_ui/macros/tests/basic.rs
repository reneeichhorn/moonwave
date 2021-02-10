use moonwave_ui_macro::*;

#[component]
pub struct BasicComponent {
  property1: usize,
  property2: usize,
}

#[component]
pub struct Foo;
#[component]
impl Foo {
  pub fn render(&self) {
    vec![]
  }
}

#[component]
pub struct Bar {
  property: usize,
  eval_property: String,
}
#[component]
impl Bar {
  pub fn render(&self) {
    vec![]
  }
}
#[component]
pub struct Parent;
#[component]
impl Parent {
  pub fn render(&self) {
    vec![]
  }
}

#[component]
impl BasicComponent {
  pub fn render(&self) -> Option<usize> {
    let something = self.property1 * 2;
    if something == 0 {
      return block! { Foo, Bar };
    } else if something == 1 {
      return vec![];
    }

    let item_layout1 = moonwave_ui::RelativeLayouterOptions {
      left: Some(moonwave_ui::SizeUnit::Pixels(0.0)),
      right: Some(moonwave_ui::SizeUnit::Pixels(0.0)),
      ..Default::default()
    };
    let item_layout2 = moonwave_ui::RelativeLayouterOptions {
      right: Some(moonwave_ui::SizeUnit::Pixels(0.0)),
      width: Some(moonwave_ui::SizeUnit::Pixels(120.0)),
      ..Default::default()
    };

    block! {
      Foo(@item_layout1),
      Bar(@item_layout2, property: self.property2),
      Bar(@item_layout2, eval_property: "1234".to_string()),
      Parent {
        Foo,
        Foo,
      }
    }
  }
}

#[test]
fn base() {}
