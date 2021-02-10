use moonwave_common::*;

use crate::{ShaderGraphNodeBuilder, ShaderType};

pub fn create_constant_vec3(value: Vector3<f32>) -> ShaderGraphNodeBuilder {
  ShaderGraphNodeBuilder::new(Box::new(move |_, outputs| {
    format!(
      "vec3 {} = vec3({:.7}, {:.7}, {:.7});",
      outputs[0].1, value.x, value.y, value.z
    )
  }))
  .add_output("output", ShaderType::Float3)
}

pub fn create_constant_vec4(value: Vector4<f32>) -> ShaderGraphNodeBuilder {
  ShaderGraphNodeBuilder::new(Box::new(move |_, outputs| {
    format!(
      "vec4 {} = vec4({:.7}, {:.7}, {:.7}, {:.7});",
      outputs[0].1, value.x, value.y, value.z, value.w
    )
  }))
  .add_output("output", ShaderType::Float4)
}

pub fn create_multiply(ty: ShaderType) -> ShaderGraphNodeBuilder {
  ShaderGraphNodeBuilder::new(Box::new(move |inputs, outputs| {
    format!(
      "{} {} = {} * {};",
      ty.get_glsl_type(),
      outputs[0].1,
      inputs[0].1,
      inputs[1].1
    )
  }))
  .add_output("output", ty)
  .add_input("x", ty)
  .add_input("y", ty)
}

pub fn create_extend1(ty: ShaderType, value: f32) -> ShaderGraphNodeBuilder {
  ShaderGraphNodeBuilder::new(Box::new(move |inputs, outputs| {
    format!(
      "{} {} = {}({}, {:.7});",
      ty.get_glsl_type(),
      outputs[0].1,
      ty.get_glsl_type(),
      inputs[0].1,
      value
    )
  }))
  .add_output("output", ty)
  .add_input("input", ty)
}
