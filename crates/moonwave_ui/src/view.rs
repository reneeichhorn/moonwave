use lyon::{
  geom::{
    euclid::{Point2D, Size2D},
    Point, Rect,
  },
  lyon_tessellation::{
    geometry_builder::simple_builder, FillOptions, FillTessellator, GeometryBuilder,
  },
  path::{builder::BorderRadii, traits::PathBuilder},
};

use crate::{
  Allocator, ChildrenCollectionProxy, ChildrenProxy, ColoredShapeGeometry, Component,
  HostedComponentRc, LayoutProps, UpdateList, SHAPE_MANAGER,
};

pub struct View {
  border_radius: (f32, f32, f32, f32),
  _border_width: (f32, f32, f32, f32),
  background_color: (f32, f32, f32, f32),
  opacity: f32,
  proxy: Option<HostedComponentRc>,
  layout: LayoutProps,
}

impl View {
  pub fn new() -> Self {
    Self {
      border_radius: (0.0, 0.0, 0.0, 0.0),
      _border_width: (0.0, 0.0, 0.0, 0.0),
      background_color: (1.0, 1.0, 1.0, 1.0),
      opacity: 1.0,
      layout: Default::default(),
      proxy: None,
    }
  }

  pub fn border_radius(mut self, radius: f32) -> Self {
    self.border_radius = (radius, radius, radius, radius);
    self
  }

  pub fn background(mut self, color: (f32, f32, f32, f32)) -> Self {
    self.background_color = color;
    self
  }

  pub fn opacity(mut self, opacity: f32) -> Self {
    self.opacity = opacity;
    self
  }
}

impl Component for View {
  fn get_layout_props(&self) -> &LayoutProps {
    &self.layout
  }
  fn get_layout_props_mut(&mut self) -> &mut LayoutProps {
    &mut self.layout
  }
  fn create(&mut self, alloc: &mut Allocator) -> Option<ChildrenProxy> {
    let proxy = alloc.alloc(ChildrenCollectionProxy {});
    self.proxy = Some(proxy.clone());
    Some(ChildrenProxy { component: proxy })
  }
  fn update(&mut self, _updates: Box<dyn UpdateList>) {}
  fn offer_layout(&self, size: (f32, f32)) -> (f32, f32) {
    size
  }
  fn mount(&mut self, size: (f32, f32), position: (f32, f32)) {
    // Prepare lyon geometry
    let mut geometry = ColoredShapeGeometry::new();
    let mut geometry_builder = simple_builder(&mut geometry);

    let mut tesselator = FillTessellator::new();
    let options = FillOptions::tolerance(0.1);
    let mut builder = tesselator.builder(&options, &mut geometry_builder);

    // Build geometry
    builder.add_rounded_rectangle(
      &Rect::new(
        Point2D::new(position.0, position.1),
        Size2D::new(size.0, size.1),
      ),
      &BorderRadii {
        top_left: self.border_radius.0,
        top_right: self.border_radius.1,
        bottom_left: self.border_radius.2,
        bottom_right: self.border_radius.3,
      },
      lyon::path::Winding::Negative,
    );
    builder.build().unwrap();

    // Register shape with renderer.
    SHAPE_MANAGER.add_colored_shape(self.background_color.into(), geometry);
  }
}
