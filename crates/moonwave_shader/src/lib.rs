#![allow(clippy::new_without_default)]

use moonwave_resources::*;

#[doc(hidden)]
pub use crevice::internal;
#[doc(hidden)]
pub use crevice::std140::{self, AsStd140, Std140};

pub use moonwave_shader_macro::{uniform, vertex};

mod base;
mod graph;

pub use base::*;
pub use graph::*;
pub use uuid::Uuid;

#[cfg(test)]
mod test;

/// Describes a type available within shaders.
#[derive(Clone, Debug, Copy)]
pub enum ShaderType {
  Matrix4,
  Float4,
  Float3,
  Float2,
  Float,
  UInt4,
  UInt3,
  UInt2,
  UInt,
  Struct(&'static str),
  Array(&'static str, usize),
}
impl ShaderType {
  /// Returns the type name in GLSL.
  pub fn get_glsl_type(&self) -> String {
    match self {
      ShaderType::Matrix4 => "mat4".to_string(),
      ShaderType::Float4 => "vec4".to_string(),
      ShaderType::Float3 => "vec3".to_string(),
      ShaderType::Float2 => "vec2".to_string(),
      ShaderType::Float => "float".to_string(),
      ShaderType::UInt4 => "uvec4".to_string(),
      ShaderType::UInt3 => "uvec3".to_string(),
      ShaderType::UInt2 => "uvec2".to_string(),
      ShaderType::UInt => "uint".to_string(),
      ShaderType::Struct(name) => name.to_string(),
      ShaderType::Array(name, _size) => name.to_string(),
    }
  }

  pub fn get_glsl_var(&self, name: &str) -> String {
    match self {
      ShaderType::Array(_name, size) => format!("{}[{}]", name, size),
      _ => name.to_string(),
    }
  }
}
impl From<VertexAttributeFormat> for ShaderType {
  fn from(org: VertexAttributeFormat) -> Self {
    match org {
      VertexAttributeFormat::Float4 => ShaderType::Float4,
      VertexAttributeFormat::Float3 => ShaderType::Float3,
      VertexAttributeFormat::Float2 => ShaderType::Float2,
      VertexAttributeFormat::Float => ShaderType::Float,
      VertexAttributeFormat::UInt4 => ShaderType::UInt4,
      VertexAttributeFormat::UInt3 => ShaderType::UInt3,
      VertexAttributeFormat::UInt2 => ShaderType::UInt2,
      VertexAttributeFormat::UInt => ShaderType::UInt,
    }
  }
}

/// Describes a sized struct that is used as a vertex buffer.
pub trait VertexStruct: Sized {
  fn generate_raw_u8(slice: &[Self]) -> &[u8];
  fn generate_attributes() -> Vec<VertexAttribute>;
  fn generate_buffer() -> VertexBuffer;
}

/// Describes a sized sturct that can be used as a uniform.
pub trait UniformStruct: Sized {
  fn get_id() -> Uuid;
  fn generate_raw_u8(&self) -> Vec<u8>;
  fn generate_name() -> String;
  fn generate_dependencies() -> Vec<(String, Vec<(String, ShaderType)>)>;
  fn generate_attributes() -> Vec<(String, ShaderType)>;
}
