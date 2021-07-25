use moonwave_common::*;

use crate::{Mesh, MeshIndex, MeshVertex, Transform};

#[derive(Debug, Clone)]
pub enum BoundingShape {
  AABB {
    min: Vector3<f32>,
    max: Vector3<f32>,
  },
}

impl BoundingShape {
  pub fn new<T: MeshVertex, I: MeshIndex>(
    mesh: &Mesh<T, I>,
    transform: Option<&Transform>,
  ) -> Self {
    // Generate matrix.
    let matrix = if let Some(transform) = transform {
      let transform = transform.get();
      let translation = Matrix4::from_translation(transform.position);
      let rotation = Matrix4::from_angle_x(Rad(transform.rotation.x))
        * Matrix4::from_angle_y(Rad(transform.rotation.y))
        * Matrix4::from_angle_z(Rad(transform.rotation.z));
      let scale =
        Matrix4::from_nonuniform_scale(transform.scale.x, transform.scale.y, transform.scale.z);
      translation * rotation * scale
    } else {
      Matrix4::identity()
    };

    // Find bounds.
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut min_z = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    let mut max_z = f32::MIN;

    for vertex in mesh.iter_vertices() {
      let position = vertex.get_position();
      let world_space = matrix * Vector4::new(position.x, position.y, position.z, 1.0);
      let world_space_norm = world_space.xyz() / world_space.w;
      min_x = min_x.min(world_space_norm.x);
      min_y = min_y.min(world_space_norm.y);
      min_z = min_z.min(world_space_norm.z);
      max_x = max_x.max(world_space_norm.x);
      max_y = max_y.max(world_space_norm.y);
      max_z = max_z.max(world_space_norm.z);
    }

    // Build AABB
    BoundingShape::AABB {
      min: Vector3::new(min_x, min_y, min_z),
      max: Vector3::new(max_x, max_y, max_z),
    }
  }

  pub fn plane_distance(plane: &Vector4<f32>, target: &Vector3<f32>) -> f32 {
    plane.w + plane.xyz().dot(*target)
  }

  pub fn vertex_positive(
    min: &Vector3<f32>,
    max: &Vector3<f32>,
    vertex: &Vector3<f32>,
  ) -> Vector3<f32> {
    let mut out = *min;
    if vertex.x > 0.0 {
      out.x = max.x
    };
    if vertex.y > 0.0 {
      out.y = max.y
    };
    if vertex.z > 0.0 {
      out.z = max.z
    };
    out
  }

  pub fn vertex_negative(
    min: &Vector3<f32>,
    max: &Vector3<f32>,
    vertex: &Vector3<f32>,
  ) -> Vector3<f32> {
    let mut out = *min;
    if vertex.x < 0.0 {
      out.x = max.x
    };
    if vertex.y < 0.0 {
      out.y = max.y
    };
    if vertex.z < 0.0 {
      out.z = max.z
    };
    out
  }

  pub fn visible_in_frustum(&self, frustum_planes: &[Vector4<f32>; 6]) -> bool {
    match self {
      BoundingShape::AABB { min, max } => {
        for plane in frustum_planes {
          let positive = Self::vertex_positive(min, max, &plane.xyz());
          if Self::plane_distance(plane, &positive) < 0.0 {
            return false;
          }
        }
        true
      }
    }
  }
}
