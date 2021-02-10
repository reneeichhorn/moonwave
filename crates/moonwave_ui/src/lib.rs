use std::sync::Arc;
use std::sync::RwLock;

mod layout;
pub use layout::*;

/// Describes a renderable UI component.
pub trait Component {
  /// Handles an incoming action
  fn handle_action(&mut self, _action: Box<dyn std::any::Any>) {}
  /// Renders the whole tree.
  fn full_render(&self) -> Vec<AnyComponentRef>;
  /// Layouts its own tree.
  fn layout(&mut self, parent: &LayouterResult);
  /// Requests that the specific component is being put on to the screen for the first time.
  fn mount(&self) {}
  /// Requests that the specific component is being removed from the screen.
  fn unmount(&self) {}
}

/// Describes a reference to a component
pub type AnyComponentRef = Arc<RwLock<dyn Component>>;
