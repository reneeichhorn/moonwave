/// Describes a mechanism that is able to layout any elements
pub trait Layouter<T: Default + Send + Sync + 'static> {
  fn evaluation(&self, options: &T, parent: &LayouterResult) -> LayouterResult;
}

/// Describes a layouter that can evaluate based on an any type.
pub trait AnyLayouter {
  fn evaluate(&self, options: &Box<dyn std::any::Any>, parent: &LayouterResult) -> LayouterResult;
}

#[derive(Clone)]
/// The unit that describes position and dimensions.
pub enum SizeUnit {
  /// Raw unit measured in pixels.
  Pixels(f32),
  /// Unit measured in percantage in realtion to the active parent.
  Percent(f32),
}
impl SizeUnit {
  pub fn in_relation_to(&self, parent: f32) -> f32 {
    match self {
      SizeUnit::Pixels(p) => *p,
      SizeUnit::Percent(p) => *p * parent,
    }
  }
}
impl Default for SizeUnit {
  fn default() -> Self {
    Self::Pixels(0.0)
  }
}

#[derive(Default, Clone)]
/// The output of a layouter execution.
pub struct LayouterResult {
  pub width: f32,
  pub height: f32,
  pub x: f32,
  pub y: f32,
}

#[derive(Default)]
pub struct RelativeLayouter;

#[derive(Default, Clone)]
pub struct RelativeLayouterOptions {
  pub left: Option<SizeUnit>,
  pub right: Option<SizeUnit>,
  pub top: Option<SizeUnit>,
  pub bottom: Option<SizeUnit>,
  pub width: Option<SizeUnit>,
  pub height: Option<SizeUnit>,
}

impl AnyLayouter for RelativeLayouter {
  fn evaluate(&self, options: &Box<dyn std::any::Any>, parent: &LayouterResult) -> LayouterResult {
    let options = options.downcast_ref::<RelativeLayouterOptions>().unwrap();
    self.evaluation(&*options, parent)
  }
}

impl Layouter<RelativeLayouterOptions> for RelativeLayouter {
  fn evaluation(
    &self,
    options: &RelativeLayouterOptions,
    parent: &LayouterResult,
  ) -> LayouterResult {
    // Evaluate horizontal coodinate
    let (x, width) = match (&options.left, &options.right, &options.width) {
      (Some(left), _, Some(width)) => (
        left.in_relation_to(parent.width),
        width.in_relation_to(parent.width),
      ),
      (None, Some(right), Some(width)) => (
        parent.width - right.in_relation_to(parent.width),
        width.in_relation_to(parent.width),
      ),
      (Some(left), Some(right), None) => {
        let x = left.in_relation_to(parent.width);
        (x, parent.width - right.in_relation_to(parent.width) - x)
      }
      _ => (0.0, 0.0),
    };
    // Evaluate vertical coodinate
    let (y, height) = match (&options.top, &options.bottom, &options.height) {
      (Some(top), _, Some(height)) => (
        top.in_relation_to(parent.height),
        height.in_relation_to(parent.height),
      ),
      (None, Some(bottom), Some(height)) => (
        parent.height - bottom.in_relation_to(parent.height),
        height.in_relation_to(parent.height),
      ),
      (Some(top), Some(bottom), None) => {
        let y = top.in_relation_to(parent.height);
        (y, parent.height - bottom.in_relation_to(parent.height) - y)
      }
      _ => (0.0, 0.0),
    };
    // Build output
    LayouterResult {
      x: parent.x + x,
      y: parent.y + y,
      width,
      height,
    }
  }
}
