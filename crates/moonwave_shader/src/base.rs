use crate::*;

#[derive(Debug)]
pub struct Constant(Vec<f32>, ShaderType);
impl Constant {
  pub const OUTPUT: usize = 0;

  pub fn new<V: std::ops::Index<std::ops::RangeFull, Output = [f32]>>(value: V) -> Self {
    let values = value.index(..).to_vec();
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

#[derive(Debug)]
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

#[derive(Debug)]
pub struct ConvertHomgenous;
impl ConvertHomgenous {
  pub const INPUT: usize = 0;
  pub const OUTPUT: usize = 0;

  pub fn new() -> Self {
    Self {}
  }
}
impl ShaderNode for ConvertHomgenous {
  fn get_outputs(&self) -> Vec<ShaderType> {
    vec![ShaderType::Float3]
  }
  fn generate(&self, inputs: &[Option<String>], outputs: &[Option<String>], output: &mut String) {
    *output += format!(
      "vec3 {} = {}.xyz / {}.w;\n",
      outputs[Self::OUTPUT].as_ref().unwrap(),
      inputs[Self::INPUT].as_ref().unwrap(),
      inputs[Self::INPUT].as_ref().unwrap(),
    )
    .as_str();
  }
}

#[derive(Debug)]
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
        .filter_map(|i| inputs[i].as_ref().cloned())
        .collect::<Vec<_>>()
        .join(",")
    )
    .as_str();
  }
}

#[derive(Debug)]
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
    for (index, ty) in self.0.iter().enumerate() {
      if let Some(output_name) = &outputs[index] {
        *output += format!(
          "{} {} = {}[{}];\n",
          ty.get_glsl_type(),
          output_name,
          inputs[Self::INPUT].as_ref().unwrap(),
          index
        )
        .as_str();
      }
    }
  }
}

#[derive(Debug)]
pub struct Vector3Upgrade;
impl Vector3Upgrade {
  pub const INPUT: usize = 0;
  pub const OUTPUT: usize = 0;
}
impl ShaderNode for Vector3Upgrade {
  fn get_outputs(&self) -> Vec<ShaderType> {
    vec![ShaderType::Float4]
  }

  fn generate(&self, inputs: &[Option<String>], outputs: &[Option<String>], output: &mut String) {
    *output += format!(
      "vec4 {} = vec4({}, 1.0);\n",
      outputs[Self::OUTPUT].as_ref().unwrap(),
      inputs[Self::INPUT].as_ref().unwrap()
    )
    .as_str();
  }
}

#[derive(Debug)]
pub struct ArrayAccess {
  output: ShaderType,
}
impl ArrayAccess {
  pub const INPUT_ARRAY: usize = 0;
  pub const INPUT_INDEX: usize = 1;
  pub const OUTPUT: usize = 0;
  pub fn new(ty: ShaderType) -> Self {
    Self { output: ty }
  }
}

impl ShaderNode for ArrayAccess {
  fn get_outputs(&self) -> Vec<ShaderType> {
    vec![self.output]
  }

  fn generate(&self, inputs: &[Option<String>], outputs: &[Option<String>], output: &mut String) {
    *output += format!(
      "{} {} = {}[{}]",
      self.output.get_glsl_type(),
      outputs[Self::OUTPUT].as_ref().unwrap(),
      inputs[Self::INPUT_ARRAY].as_ref().unwrap(),
      inputs[Self::INPUT_INDEX].as_ref().unwrap()
    )
    .as_str();
  }
}

#[derive(Debug)]
pub struct MemberAccess {
  output: ShaderType,
  name: String,
}
impl MemberAccess {
  pub const INPUT_STRUCT: usize = 0;
  pub const INPUT_NAME: usize = 1;
  pub const OUTPUT: usize = 0;
  pub fn new(name: String, output: ShaderType) -> Self {
    Self { name, output }
  }
}

impl ShaderNode for MemberAccess {
  fn get_outputs(&self) -> Vec<ShaderType> {
    vec![self.output]
  }

  fn generate(&self, inputs: &[Option<String>], outputs: &[Option<String>], output: &mut String) {
    *output += format!(
      "{} {} = {}.{}",
      self.output.get_glsl_type(),
      outputs[Self::OUTPUT].as_ref().unwrap(),
      inputs[Self::INPUT_STRUCT].as_ref().unwrap(),
      self.name,
    )
    .as_str();
  }
}
