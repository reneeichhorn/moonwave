use generational_arena::Arena;
use moonwave_resources::VertexAttribute;
use thiserror::Error;

use crate::{ShaderType, UniformStruct, VertexStruct};

pub use generational_arena::Index;

const MAX_NODES: usize = 128;
const MAX_INPUT_OUTPUTS_PER_NODE: usize = 16;

pub struct ShaderGraph {
  vertex_attributes: Vec<VertexAttribute>,
  vertex_output_node: Option<Index>,
  color_outputs: Vec<(String, ShaderType, Index)>,
  uniforms: Vec<Uniform>,
  nodes: Arena<Node>,
}

impl ShaderGraph {
  pub fn new() -> Self {
    Self {
      nodes: Arena::with_capacity(MAX_NODES),
      color_outputs: Vec::new(),
      vertex_attributes: Vec::new(),
      uniforms: Vec::new(),
      vertex_output_node: None,
    }
  }

  pub fn add_vertex_attributes<T: VertexStruct + 'static>(&mut self) -> (Index, Index) {
    self.vertex_attributes = T::generate_attributes();
    let index = self.add_node(VertexAttributesNode {
      attributes: self.vertex_attributes.clone(),
    });
    let output_node = self.add_node(VertexShaderOutputNode {});
    self.vertex_output_node = Some(output_node);
    (index, output_node)
  }

  pub fn add_color_output(&mut self, name: &str, format: ShaderType) -> Index {
    let string = name.to_string();
    let index = self.add_node(ColorOutputNode {
      name: string.clone(),
    });
    self.color_outputs.push((string, format, index));
    index
  }

  pub fn add_uniform<T: UniformStruct>(&mut self) -> (usize, Index) {
    let node = UniformNode {
      name: T::generate_name(),
      attributes: T::generate_attributes(),
    };
    let index = self.add_node(node);
    self.uniforms.push(Uniform {
      node_index: index,
      name: T::generate_name(),
      attributes: T::generate_attributes(),
    });
    (self.uniforms.len() - 1, index)
  }

  /// Add a new node into the graph.
  pub fn add_node<T: ShaderNode>(&mut self, node: T) -> Index {
    self.nodes.insert(Node {
      node: Box::new(node),
      inputs: [None; MAX_INPUT_OUTPUTS_PER_NODE],
    })
  }

  /// Connects one nodes output to another nodes input.
  pub fn connect(
    &mut self,
    source: Index,
    source_output: usize,
    destination: Index,
    destination_input: usize,
  ) -> Result<(), GraphConnectError> {
    // Validate connection parameters.
    if destination_input >= MAX_INPUT_OUTPUTS_PER_NODE {
      return Err(GraphConnectError::MaximumInputsReached);
    };
    if source_output >= MAX_INPUT_OUTPUTS_PER_NODE {
      return Err(GraphConnectError::MaximumInputsReached);
    };

    let destination_node = self
      .nodes
      .get_mut(destination)
      .ok_or(GraphConnectError::InvalidDestination)?;

    // Target input is already connected.
    if destination_node.inputs[destination_input].is_some() {
      return Err(GraphConnectError::AlreadyConnected);
    }

    // Target input is empty so simply create the connection.
    destination_node.inputs[destination_input] = Some(Input {
      owner_node_index: source,
      owner_node_output: source_output,
    });

    Ok(())
  }

  pub fn build(&self, outputs: &[Index]) -> BuiltShaderGraph {
    let mut vertex_shader_code = String::with_capacity(1024 * 1024);
    let mut fragment_shader_code = String::with_capacity(1024 * 1024);

    // Traverse from the starting point of the vertex output node.
    let traversed_vertex_shader = {
      optick::event!("ShaderGraph::traverse_vertex_shader");
      let mut nodes = Vec::with_capacity(MAX_NODES);
      self.traverse(self.vertex_output_node.unwrap(), &mut nodes);
      Self::dedup_unordered(&nodes)
    };

    // Traverse from the starting point of each color output.
    let mut traversed_fragment_shader = {
      optick::event!("ShaderGraph::traverse_fragment_shader");
      let mut nodes = Vec::with_capacity(MAX_NODES);
      for node in outputs {
        self.traverse(*node, &mut nodes);
      }
      Self::dedup_unordered(&nodes)
    };

    // Find shared subtrees to optimze fragment shader with vertex shader calculations.
    let shared_nodes = {
      optick::event!("ShaderGraph::vs_fs_subtrees");
      let mut nodes = Vec::with_capacity(16);

      // Find shared subtrees for all color outputs.
      for (_, _, node) in &self.color_outputs {
        self.traverse_subtree(
          &traversed_vertex_shader,
          &traversed_fragment_shader,
          &mut nodes,
          *node,
        );
      }
      Self::dedup_unordered(&nodes)
    };

    // Build vertex / frag shared variables.
    let shared_attributes = {
      optick::event!("ShaderGraph::shared_attributes");
      let mut attributes = Vec::with_capacity(32);
      for node_index in &shared_nodes {
        // Never share uniform nodes directly - uniforms can already be shared within binding easier and faster.
        if self.uniforms.iter().any(|u| u.node_index == *node_index) {
          continue;
        }

        let node = self.nodes.get(*node_index).unwrap();
        let outputs = node.node.get_outputs();
        for (index, output_ty) in outputs.iter().enumerate() {
          attributes.push((
            *output_ty,
            format!("var_{}_{}", node_index.into_raw_parts().0, index),
          ));
        }
      }
      attributes
    };

    // Uniforms
    let uniforms = self
      .uniforms
      .iter()
      .enumerate()
      .map(|(index, uniform)| BuiltUniform {
        binding: index,
        name: uniform.name.clone(),
        attributes: uniform.attributes.clone(),
        in_vs: traversed_vertex_shader.contains(&uniform.node_index),
        in_fs: traversed_fragment_shader.contains(&uniform.node_index)
          && !shared_nodes.contains(&uniform.node_index),
      })
      .collect::<Vec<_>>();

    // Remove unneded uniform nodes out.
    for (index, uniform) in uniforms.iter().enumerate() {
      if uniform.in_fs {
        continue;
      }
      traversed_fragment_shader.retain(|node| node != &self.uniforms[index].node_index);
    }

    // Vertex shader.
    {
      optick::event!("ShaderGraph::generate_vertex_shader");
      vertex_shader_code += "#version 450\n\n";

      // Vertex attributes
      for attr in &self.vertex_attributes {
        vertex_shader_code += format!(
          "layout (location = {}) in {} a_{};\n",
          attr.location,
          ShaderType::from(attr.format).get_glsl_type(),
          attr.name,
        )
        .as_str();
      }

      // Shared attributes for fragment shader.
      for (index, (ty, name)) in shared_attributes.iter().enumerate() {
        vertex_shader_code += format!(
          "layout (location = {}) out {} vs_{};\n",
          index,
          ty.get_glsl_type(),
          name
        )
        .as_str();
      }

      // Uniforms
      for uniform in &uniforms {
        if !uniform.in_vs {
          continue;
        }
        Self::generate_uniform(uniform, &mut vertex_shader_code);
      }

      // Code
      vertex_shader_code += "void main() {\n";
      self.generate_code(&mut vertex_shader_code, &traversed_vertex_shader, &[]);
      for (_, name) in &shared_attributes {
        vertex_shader_code += format!("vs_{} = {};\n", name, name).as_str();
      }
      vertex_shader_code += "}\n";
    }

    // Fragment shader
    {
      optick::event!("ShaderGraph::generate_fragment_shader");
      fragment_shader_code += "#version 450\n\n";

      // Shared attributes for fragment shader.
      for (index, (ty, name)) in shared_attributes.iter().enumerate() {
        fragment_shader_code += format!(
          "layout (location = {}) in {} vs_{};\n",
          index,
          ty.get_glsl_type(),
          name
        )
        .as_str();
      }

      // Color outputs
      for (index, (name, ty, _)) in self
        .color_outputs
        .iter()
        .filter(|(_, _, node_index)| outputs.contains(node_index))
        .enumerate()
      {
        fragment_shader_code += format!(
          "layout (location = {}) out {} f_{};\n",
          index,
          ty.get_glsl_type(),
          name
        )
        .as_str();
      }

      // Uniforms
      for uniform in &uniforms {
        if !uniform.in_fs {
          continue;
        }
        Self::generate_uniform(uniform, &mut fragment_shader_code);
      }

      // Code
      fragment_shader_code += "void main() {\n";
      for (ty, name) in &shared_attributes {
        fragment_shader_code +=
          format!("{} {} = vs_{};\n", ty.get_glsl_type(), name, name).as_str();
      }
      let vs = traversed_vertex_shader
        .iter()
        .copied()
        .filter(|it| self.uniforms.iter().find(|u| &u.node_index == it).is_none())
        .collect::<Vec<_>>();
      self.generate_code(&mut fragment_shader_code, &traversed_fragment_shader, &vs);
      fragment_shader_code += "}\n";
    }

    BuiltShaderGraph {
      vs: vertex_shader_code,
      fs: fragment_shader_code,
      required_uniforms: uniforms,
    }
  }

  fn generate_uniform(uniform: &BuiltUniform, output: &mut String) {
    *output += format!(
      "layout (set = {}, binding = 0) uniform {}_block {{\n",
      uniform.binding, uniform.name
    )
    .as_str();

    for attr in &uniform.attributes {
      *output += format!("\t{} {};\n", attr.1.get_glsl_type(), attr.0).as_str();
    }

    *output += format!("}} {};\n", uniform.name).as_str();
  }

  fn generate_code(&self, output: &mut String, nodes: &[Index], skipped: &[Index]) {
    for node_index in nodes.iter().rev() {
      if skipped.contains(node_index) {
        continue;
      }

      let node = self.nodes.get(*node_index).unwrap();

      let inputs = node
        .inputs
        .iter()
        .map(|input| {
          input.map(|input| {
            format!(
              "var_{}_{}",
              input.owner_node_index.into_raw_parts().0,
              input.owner_node_output
            )
          })
        })
        .collect::<Vec<_>>();

      let outputs = (0..MAX_INPUT_OUTPUTS_PER_NODE)
        .into_iter()
        .map(|i| Some(format!("var_{}_{}", node_index.into_raw_parts().0, i)))
        .collect::<Vec<_>>();

      node.node.generate(&inputs, &outputs, output);
    }
  }

  fn dedup_unordered(list: &[Index]) -> Vec<Index> {
    let mut deduped = Vec::new();
    for item in list {
      if deduped.contains(item) {
        continue;
      }
      deduped.push(*item);
    }
    deduped
  }

  fn traverse_subtree(
    &self,
    nodes_source: &[Index],
    nodes_target: &[Index],
    shared: &mut Vec<Index>,
    current: Index,
  ) {
    let node = self.nodes.get(current).unwrap();
    for input in &node.inputs {
      if let Some(input) = input {
        let target = input.owner_node_index;
        if nodes_source.contains(&target) && nodes_target.contains(&target) {
          shared.push(target);
        } else {
          self.traverse_subtree(nodes_source, nodes_target, shared, target);
        }
      }
    }
  }

  fn traverse(&self, index: Index, output: &mut Vec<Index>) {
    let node = self.nodes.get(index).unwrap();
    output.push(index);

    for input in &node.inputs {
      if let Some(input) = input {
        self.traverse(input.owner_node_index, output);
      }
    }
  }
}

struct Node {
  inputs: [Option<Input>; MAX_INPUT_OUTPUTS_PER_NODE],
  node: Box<dyn ShaderNode>,
}
#[derive(Copy, Clone)]
struct Input {
  owner_node_index: Index,
  owner_node_output: usize,
}

struct Uniform {
  node_index: Index,
  name: String,
  attributes: Vec<(String, ShaderType)>,
}

pub trait ShaderNode: 'static {
  fn get_available_stages(&self) -> (bool, bool) {
    (true, true)
  }

  fn get_type_expectation(&self, _index: usize) -> Option<ShaderType> {
    None
  }

  fn get_outputs(&self) -> Vec<ShaderType> {
    Vec::new()
  }

  fn generate(&self, inputs: &[Option<String>], outputs: &[Option<String>], output: &mut String);
}

#[derive(Clone)]
struct VertexAttributesNode {
  attributes: Vec<VertexAttribute>,
}

impl ShaderNode for VertexAttributesNode {
  fn get_available_stages(&self) -> (bool, bool) {
    (true, false)
  }

  fn get_outputs(&self) -> Vec<ShaderType> {
    self
      .attributes
      .iter()
      .map(|x| ShaderType::from(x.format))
      .collect::<Vec<_>>()
  }

  fn generate(&self, _inputs: &[Option<String>], outputs: &[Option<String>], output: &mut String) {
    for (index, attribute) in self.attributes.iter().enumerate() {
      *output += format!(
        "{} {} = a_{};\n",
        ShaderType::from(attribute.format).get_glsl_type(),
        outputs[index].as_ref().unwrap(),
        attribute.name
      )
      .as_str();
    }
  }
}

struct VertexShaderOutputNode;

impl ShaderNode for VertexShaderOutputNode {
  fn get_available_stages(&self) -> (bool, bool) {
    (true, false)
  }

  fn get_type_expectation(&self, _index: usize) -> Option<ShaderType> {
    Some(ShaderType::Float4)
  }

  fn generate(&self, inputs: &[Option<String>], _outputs: &[Option<String>], output: &mut String) {
    *output += format!("gl_Position = {};\n", inputs[0].as_ref().unwrap()).as_str();
  }
}

struct ColorOutputNode {
  name: String,
}

impl ShaderNode for ColorOutputNode {
  fn get_available_stages(&self) -> (bool, bool) {
    (false, true)
  }
  fn get_type_expectation(&self, _index: usize) -> Option<ShaderType> {
    Some(ShaderType::Float4)
  }
  fn generate(&self, inputs: &[Option<String>], _outputs: &[Option<String>], output: &mut String) {
    *output += format!("f_{} = {};\n", self.name, inputs[0].as_ref().unwrap()).as_str();
  }
}
struct UniformNode {
  name: String,
  attributes: Vec<(String, ShaderType)>,
}
impl ShaderNode for UniformNode {
  fn get_outputs(&self) -> Vec<ShaderType> {
    self
      .attributes
      .iter()
      .map(|(_, ty)| *ty)
      .collect::<Vec<_>>()
  }

  fn generate(&self, _inputs: &[Option<String>], outputs: &[Option<String>], output: &mut String) {
    for (index, (name, ty)) in self.attributes.iter().enumerate() {
      *output += format!(
        "{} {} = {}.{};\n",
        ty.get_glsl_type(),
        outputs[index].as_ref().unwrap(),
        self.name,
        name,
      )
      .as_str();
    }
  }
}

#[derive(Debug)]
pub struct BuiltUniform {
  pub binding: usize,
  name: String,
  attributes: Vec<(String, ShaderType)>,
  pub in_vs: bool,
  pub in_fs: bool,
}

#[derive(Debug)]
pub struct BuiltShaderGraph {
  pub vs: String,
  pub fs: String,
  pub required_uniforms: Vec<BuiltUniform>,
}

#[derive(Error, Debug)]
pub enum GraphConnectError {
  #[error("The target node has reached its input limit")]
  MaximumInputsReached,
  #[error("The source node has reached its outputs limit")]
  MaximumOutputsReached,
  #[error("The target node does not exist")]
  InvalidDestination,
  #[error("The target nodes input is already connected")]
  AlreadyConnected,
}
