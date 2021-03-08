use crate::*;
use moonwave_common::{Vector2, Vector3, Vector4};

mod moonwave_shader {
  pub use crate::*;
}

#[vertex]
struct SampleVertex {
  position: Vector3<f32>,
}

#[test]
fn test_basic_shader() {
  // Build shader structure
  let mut shader = ShaderGraph::new();
  let (vertex_in, vertex_out) = shader.add_vertex_attributes::<SampleVertex>();
  let color = shader.add_color_output("color", ShaderType::Float4);
  let normal = shader.add_color_output("normal", ShaderType::Float4);

  // Build graph.
  let const0 = shader.add_node(Constant::new_scalar(1.0));
  let const1 = shader.add_node(Constant::new(Vector3::new(0.0, 1.0, 2.0)));
  let const2 = shader.add_node(Constant::new(Vector4::new(1.0, 1.0, 1.0, 1.0)));
  let multiply = shader.add_node(Multiply::new(ShaderType::Float3));
  let deconstruct = shader.add_node(Deconstruct::new(ShaderType::Float4).unwrap());
  let construct = shader.add_node(Construct::new(ShaderType::Float4).unwrap());

  shader
    .connect(
      vertex_in,
      SampleVertex::OUTPUT_POSITION,
      multiply,
      Multiply::INPUT_A,
    )
    .unwrap();
  shader
    .connect(const1, 0, multiply, Multiply::INPUT_B)
    .unwrap();
  shader
    .connect(multiply, Multiply::OUTPUT, deconstruct, Deconstruct::INPUT)
    .unwrap();
  shader
    .connect(
      deconstruct,
      Deconstruct::OUTPUT_X,
      construct,
      Construct::INPUT_X,
    )
    .unwrap();
  shader
    .connect(
      deconstruct,
      Deconstruct::OUTPUT_Y,
      construct,
      Construct::INPUT_Y,
    )
    .unwrap();
  shader
    .connect(
      deconstruct,
      Deconstruct::OUTPUT_Z,
      construct,
      Construct::INPUT_Z,
    )
    .unwrap();
  shader
    .connect(const0, Constant::OUTPUT, construct, Construct::INPUT_W)
    .unwrap();
  shader
    .connect(construct, Construct::OUTPUT, vertex_out, 0)
    .unwrap();
  shader
    .connect(construct, Construct::OUTPUT, color, 0)
    .unwrap();
  shader.connect(const2, Constant::OUTPUT, normal, 0).unwrap();

  // Full Build
  let built = shader.build(&[color, normal]);
  let built_color = shader.build(&[color]);
  let built_normal = shader.build(&[normal]);
  insta::assert_debug_snapshot!("simple", vec![built, built_color, built_normal]);
}

#[cfg(test)]
#[uniform]
struct SampleUniformA {
  sample: f32,
  position: Vector3<f32>,
}
#[cfg(test)]
#[uniform]
struct SampleUniformB {
  sample: f32,
}

#[test]
fn test_uniform_shader() {
  // Build shader structure
  let mut shader = ShaderGraph::new();
  let (vertex_in, vertex_out) = shader.add_vertex_attributes::<SampleVertex>();
  let color = shader.add_color_output("color", ShaderType::Float4);
  let (_, uniform_a) = shader.add_uniform::<SampleUniformA>();
  let (_, uniform_b) = shader.add_uniform::<SampleUniformB>();

  // Construct
  let const0 = shader.add_node(Constant::new_scalar(1.0));
  let multiply = shader.add_node(Multiply::new(ShaderType::Float3));
  let multiply2 = shader.add_node(Multiply::new(ShaderType::Float3));
  let multiply3 = shader.add_node(Multiply::new(ShaderType::Float3));
  let deconstruct = shader.add_node(Deconstruct::new(ShaderType::Float4).unwrap());
  let construct = shader.add_node(Construct::new(ShaderType::Float4).unwrap());
  shader.connect(
    vertex_in,
    SampleVertex::OUTPUT_POSITION,
    multiply,
    Multiply::INPUT_A,
  );
  shader.connect(
    uniform_a,
    SampleUniformA::OUTPUT_POSITION,
    multiply,
    Multiply::INPUT_B,
  );
  shader.connect(multiply, Multiply::OUTPUT, multiply2, Multiply::INPUT_A);
  shader.connect(
    uniform_b,
    SampleUniformB::OUTPUT_SAMPLE,
    multiply2,
    Multiply::INPUT_B,
  );
  shader.connect(multiply2, Multiply::OUTPUT, deconstruct, Deconstruct::INPUT);
  shader
    .connect(
      deconstruct,
      Deconstruct::OUTPUT_X,
      construct,
      Construct::INPUT_X,
    )
    .unwrap();
  shader
    .connect(
      deconstruct,
      Deconstruct::OUTPUT_Y,
      construct,
      Construct::INPUT_Y,
    )
    .unwrap();
  shader
    .connect(
      deconstruct,
      Deconstruct::OUTPUT_Z,
      construct,
      Construct::INPUT_Z,
    )
    .unwrap();
  shader
    .connect(const0, Constant::OUTPUT, construct, Construct::INPUT_W)
    .unwrap();
  shader
    .connect(construct, Construct::OUTPUT, vertex_out, 0)
    .unwrap();
  shader
    .connect(construct, Construct::OUTPUT, multiply3, Multiply::INPUT_A)
    .unwrap();
  shader
    .connect(
      uniform_a,
      SampleUniformA::OUTPUT_SAMPLE,
      multiply3,
      Multiply::INPUT_B,
    )
    .unwrap();
  shader
    .connect(multiply3, Multiply::OUTPUT, color, 0)
    .unwrap();

  // Full build
  let built = shader.build(&[color]);
  insta::assert_debug_snapshot!("ub_full", built);
}
