use crate::*;
use moonwave_common::{Vector2, Vector3, Vector4};

mod moonwave_shader {
  pub use crate::*;
}

#[vertex]
struct SampleVertex {
  position: Vector3<f32>,
  uv: Vector2<f32>,
}

#[test]
fn test_basic_shader() {
  let mut shader = Shader::new()
    .add_color_output("color", ShaderType::Float4)
    .add_color_output("normal", ShaderType::Float4)
    .add_vertex_attributes::<SampleVertex>()
    .finish();

  // Build graph
  {
    let bootstrap_start = shader.get_bootstrap_node();
    let bootstrap_end = shader.get_bootstrap_end_node();

    let graph = shader.get_graph_mut();
    let const_node1 =
      graph.add_node_builder(base::create_constant_vec3(Vector3::new(0.0, 1.0, 2.0)));
    let const_node2 =
      graph.add_node_builder(base::create_constant_vec4(Vector4::new(1.0, 1.0, 1.0, 1.0)));
    let multiply = graph.add_node_builder(base::create_multiply(ShaderType::Float3));
    let extend = graph.add_node_builder(base::create_extend1(ShaderType::Float4, 1.0));

    graph
      .connect_name(bootstrap_start, "position", multiply, "x")
      .unwrap();
    graph
      .connect_name(const_node1, "output", multiply, "y")
      .unwrap();
    graph
      .connect_name(multiply, "output", extend, "input")
      .unwrap();
    graph
      .connect_name(extend, "output", bootstrap_end, "position")
      .unwrap();
    graph
      .connect_name(extend, "output", bootstrap_end, "color")
      .unwrap();
    graph
      .connect_name(const_node2, "output", bootstrap_end, "normal")
      .unwrap();
  }

  let out = shader.build_full();
  insta::assert_snapshot!("full_vs", out.0);
  insta::assert_snapshot!("full_fs", out.1);
  let color_out = shader.build(&[shader.get_color_output_entity("color")]);
  insta::assert_snapshot!("color_vs", color_out.0);
  insta::assert_snapshot!("color_fs", color_out.1);
  let normal_out = shader.build(&[shader.get_color_output_entity("normal")]);
  insta::assert_snapshot!("normal_vs", normal_out.0);
  insta::assert_snapshot!("normal_fs", normal_out.1);
}
