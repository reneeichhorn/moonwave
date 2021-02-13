use moonwave_common::{storage::ContiguousStorage, *};

use crate::*;

pub struct Constant(Vec<f32>, ShaderType);
impl Constant {
  pub const OUTPUT: usize = 0;

  pub fn new<R: Dim, C: Dim, S: ContiguousStorage<f32, R, C>>(value: Matrix<f32, R, C, S>) -> Self {
    let values = value.as_slice().to_vec();
    let ty = match values.len() {
      4 => ShaderType::Float4,
      3 => ShaderType::Float3,
      2 => ShaderType::Float2,
      _ => ShaderType::Float,
    };
    Self(values, ty)
  }

  pub fn new_scalar(value: f32) -> Self {
    Self(vec![value], ShaderType::Float)
  }
}
impl ShaderNode for Constant {
  fn get_outputs(&self) -> Vec<ShaderType> {
    vec![self.1]
  }

  fn generate(&self, _inputs: &[Option<String>], outputs: &[Option<String>], output: &mut String) {
    *output += format!(
      "{} {} = {}({});\n",
      self.1.get_glsl_type(),
      outputs[Self::OUTPUT].as_ref().unwrap(),
      self.1.get_glsl_type(),
      self
        .0
        .iter()
        .map(|x| format!("{:.7}", x))
        .collect::<Vec<_>>()
        .join(",")
    )
    .as_str();
  }
}

pub struct Multiply(ShaderType);
impl Multiply {
  pub const INPUT_A: usize = 0;
  pub const INPUT_B: usize = 1;
  pub const OUTPUT: usize = 0;

  pub fn new(ty: ShaderType) -> Self {
    Self(ty)
  }
}
impl ShaderNode for Multiply {
  fn get_outputs(&self) -> Vec<ShaderType> {
    vec![self.0]
  }
  fn generate(&self, inputs: &[Option<String>], outputs: &[Option<String>], output: &mut String) {
    *output += format!(
      "{} {} = {} * {};\n",
      self.0.get_glsl_type(),
      outputs[Self::OUTPUT].as_ref().unwrap(),
      inputs[Self::INPUT_A].as_ref().unwrap(),
      inputs[Self::INPUT_B].as_ref().unwrap()
    )
    .as_str();
  }
}

pub struct Construct(Vec<ShaderType>, ShaderType);
impl Construct {
  pub const INPUT_X: usize = 0;
  pub const INPUT_Y: usize = 1;
  pub const INPUT_Z: usize = 2;
  pub const INPUT_W: usize = 3;
  pub const OUTPUT: usize = 0;

  pub fn new(ty: ShaderType) -> Option<Self> {
    match ty {
      ShaderType::Float4 => Some(Self(vec![ShaderType::Float; 4], ShaderType::Float4)),
      ShaderType::Float3 => Some(Self(vec![ShaderType::Float; 3], ShaderType::Float3)),
      ShaderType::Float2 => Some(Self(vec![ShaderType::Float; 2], ShaderType::Float2)),
      _ => None,
    }
  }
}

impl ShaderNode for Construct {
  fn get_outputs(&self) -> Vec<ShaderType> {
    vec![self.1]
  }
  fn generate(&self, inputs: &[Option<String>], outputs: &[Option<String>], output: &mut String) {
    *output += format!(
      "{} {} = {}({});\n",
      self.1.get_glsl_type(),
      outputs[Self::OUTPUT].as_ref().unwrap(),
      self.1.get_glsl_type(),
      (0..self.0.len())
        .into_iter()
        .map(|i| format!("{:.7}", inputs[i].as_ref().unwrap()))
        .collect::<Vec<_>>()
        .join(",")
    )
    .as_str();
  }
}

pub struct Deconstruct(Vec<ShaderType>, ShaderType);
impl Deconstruct {
  pub const INPUT: usize = 0;
  pub const OUTPUT_X: usize = 0;
  pub const OUTPUT_Y: usize = 1;
  pub const OUTPUT_Z: usize = 2;
  pub const OUTPUT_W: usize = 3;

  pub fn new(ty: ShaderType) -> Option<Self> {
    match ty {
      ShaderType::Float4 => Some(Self(vec![ShaderType::Float; 4], ShaderType::Float4)),
      ShaderType::Float3 => Some(Self(vec![ShaderType::Float; 3], ShaderType::Float3)),
      ShaderType::Float2 => Some(Self(vec![ShaderType::Float; 2], ShaderType::Float2)),
      _ => None,
    }
  }
}

impl ShaderNode for Deconstruct {
  fn get_outputs(&self) -> Vec<ShaderType> {
    self.0.clone()
  }
  fn generate(&self, inputs: &[Option<String>], outputs: &[Option<String>], output: &mut String) {
    for (index, _) in self.0.iter().enumerate() {
      if let Some(output_name) = &outputs[index] {
        *output += format!(
          "{} {} = {}[{}];\n",
          self.1.get_glsl_type(),
          output_name,
          inputs[Self::INPUT].as_ref().unwrap(),
          index
        )
        .as_str();
      }
    }
  }
}
