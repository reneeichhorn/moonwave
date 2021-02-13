#![allow(clippy::new_without_default)]

use moonwave_resources::*;

pub use moonwave_shader_macro::vertex;

mod base;
mod graph;

pub use base::*;
pub use graph::*;

mod test;

/// Describes a type available within shaders.
#[derive(Clone, Debug, Copy)]
pub enum ShaderType {
  Float4,
  Float3,
  Float2,
  Float,
  UInt4,
  UInt3,
  UInt2,
  UInt,
}
impl ShaderType {
  /// Returns the type name in GLSL.
  pub fn get_glsl_type(&self) -> &'static str {
    match self {
      ShaderType::Float4 => "vec4",
      ShaderType::Float3 => "vec3",
      ShaderType::Float2 => "vec2",
      ShaderType::Float => "float",
      ShaderType::UInt4 => "uvec4",
      ShaderType::UInt3 => "uvec3",
      ShaderType::UInt2 => "uvec2",
      ShaderType::UInt => "uint",
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
