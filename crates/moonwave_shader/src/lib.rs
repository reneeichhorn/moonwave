#![allow(clippy::new_without_default)]

use generational_arena::{Arena, Index as ArenaIndex};
use moonwave_resources::*;
use thiserror::Error;

pub use moonwave_shader_macro::vertex;
pub mod base;
mod test;

/// A unbuilt shader
pub struct Shader {
  graph: ShaderGraph,
  vertex_attributes: Vec<VertexAttribute>,
  color_outputs: Vec<(String, ShaderType)>,
  node_vertex: Option<ArenaIndex>,
  node_output: Option<ArenaIndex>,
}

impl Shader {
  /// Creates new empty shader.
  pub fn new() -> Self {
    Self {
      graph: ShaderGraph::new(),
      vertex_attributes: Vec::new(),
      color_outputs: Vec::new(),
      node_output: None,
      node_vertex: None,
    }
  }

  /// Adds all vertex attributes based on a vertex struct.
  pub fn add_vertex_attributes<T: VertexStruct>(mut self) -> Self {
    self
      .vertex_attributes
      .extend(T::generate_attributes().into_iter());

    let bootstrap = GraphBootstrap {
      attributes: T::generate_attributes(),
    };
    self.node_vertex = Some(
      self
        .graph
        .add_node_builder(ShaderGraphNodeBuilder::from_generator(bootstrap)),
    );
    self
  }

  /// Adds a color output to the shader.
  pub fn add_color_output(mut self, name: &str, ty: ShaderType) -> Self {
    self.color_outputs.push((name.to_string(), ty));
    self
  }

  pub fn finish(mut self) -> Self {
    let bootstrap = GraphBootstrapEnd {
      outputs: self.color_outputs.clone(),
    };
    self.node_output = Some(
      self
        .graph
        .add_node_builder(ShaderGraphNodeBuilder::from_generator(bootstrap)),
    );
    self
  }

  pub fn get_graph_mut(&mut self) -> &mut ShaderGraph {
    &mut self.graph
  }

  pub fn get_input_entity_by_name(&self, node: ArenaIndex, name: &str) -> Option<ArenaIndex> {
    self
      .graph
      .nodes
      .get(node)
      .unwrap()
      .inputs
      .iter()
      .find(|i| self.graph.entities.get(**i).unwrap().name == name)
      .copied()
  }

  fn build_function_code(
    &self,
    nodes: &[ArenaIndex],
    ignored_nodes: &[ArenaIndex],
    fragment_entities: &[ArenaIndex],
    is_fragment: bool,
  ) -> String {
    let mut code = String::with_capacity(1024);
    for node_index in nodes.iter().rev() {
      if ignored_nodes.contains(node_index) {
        continue;
      }

      let node = self.graph.nodes.get(*node_index).unwrap();

      let inputs = node
        .inputs
        .iter()
        .enumerate()
        .filter_map(|(index, input_index)| {
          if *node_index == self.get_bootstrap_end_node() && index == 0 && is_fragment {
            return None;
          }
          if *node_index == self.get_bootstrap_end_node() && index > 0 && !is_fragment {
            return None;
          }

          let source_index = {
            let input = self.graph.entities.get(*input_index).unwrap();
            input.connection.first().copied().unwrap()
          };
          let source = self.graph.entities.get(source_index).unwrap();
          if *node_index == self.get_bootstrap_end_node()
            && is_fragment
            && !fragment_entities.contains(input_index)
          {
            return None;
          }

          Some((
            index,
            format!(
              "node_{}_{}",
              source.parent_node.unwrap().into_raw_parts().0,
              source.name
            ),
          ))
        })
        .collect::<Vec<_>>();

      let outputs = node
        .outputs
        .iter()
        .enumerate()
        .map(|(index, input_index)| {
          let input = self.graph.entities.get(*input_index).unwrap();
          (
            index,
            format!("node_{}_{}", node_index.into_raw_parts().0, input.name),
          )
        })
        .collect::<Vec<_>>();

      code += textwrap::indent(
        format!("{}\n", (node.generator)(&inputs, &outputs)).as_str(),
        "  ",
      )
      .as_str();
    }
    code
  }

  pub fn get_color_output_entity(&self, name: &str) -> ArenaIndex {
    self
      .get_input_entity_by_name(self.node_output.unwrap(), name)
      .unwrap()
  }

  pub fn build_full(&self) -> (String, String) {
    let list = self
      .color_outputs
      .iter()
      .map(|(name, _)| {
        self
          .get_input_entity_by_name(self.node_output.unwrap(), name.as_str())
          .unwrap()
      })
      .collect::<Vec<_>>();
    self.build(&list)
  }

  pub fn build(&self, list: &[ArenaIndex]) -> (String, String) {
    //let vertex_nodes = Vec::new();
    //let fragment_nodes = Vec::new();
    let mut vertex_shader = String::with_capacity(1024);
    let mut fragment_shader = String::with_capacity(1024);

    // Vertex shader reverse traverse.
    let vertex_shader_traversed = {
      let position_entity = self
        .get_input_entity_by_name(self.node_output.unwrap(), "position")
        .unwrap();
      self.graph.traverse(&[position_entity])
    };

    // Fragment shader reverse traverse.
    let fragment_shader_traversed = self.graph.traverse(list);

    // Calculate shared nodes between vertex and fragment shader
    let shared = self
      .graph
      .find_shared_subtrees(&vertex_shader_traversed, &fragment_shader_traversed);

    // Build attributes
    let shared_attributes = shared
      .iter()
      .flat_map(|out| {
        let node = self.graph.nodes.get(*out).unwrap();
        node.outputs.iter().map(|output| {
          let entity = self.graph.entities.get(*output).unwrap();
          (
            entity.ty,
            format!(
              "node_{}_{}",
              entity.parent_node.unwrap().into_raw_parts().0,
              entity.name
            ),
          )
        })
      })
      .collect::<Vec<_>>();

    // Vertex shader
    {
      // Vertex attributes
      for attr in &self.vertex_attributes {
        vertex_shader += format!(
          "layout (location = {}) in {} a_{};\n",
          attr.location,
          ShaderType::from(attr.format).get_glsl_type(),
          attr.name,
        )
        .as_str();
      }

      // Shared attributes for fragment shader.
      for (index, (ty, name)) in shared_attributes.iter().enumerate() {
        vertex_shader += format!(
          "layout (location = {}) out {} vs_{};\n",
          index,
          ty.get_glsl_type(),
          name
        )
        .as_str();
      }

      // Build main function
      vertex_shader += "void main() {\n";
      vertex_shader += self
        .build_function_code(&vertex_shader_traversed, &Vec::new(), &Vec::new(), false)
        .as_str();
      // Assignment of vertex outputs.
      for (_ty, name) in shared_attributes.iter() {
        vertex_shader += format!("  vs_{} = {};\n", name, name).as_str();
      }

      vertex_shader += "}\n";
    }

    // Fragment shader
    {
      // Shared attributes from vertex shader.
      for (index, (ty, name)) in shared_attributes.iter().enumerate() {
        fragment_shader += format!(
          "layout (location = {}) in {} vs_{};\n",
          index,
          ty.get_glsl_type(),
          name
        )
        .as_str();
      }

      // Color outputs
      for (index, (name, ty)) in self.color_outputs.iter().enumerate() {
        fragment_shader += format!(
          "layout (location = {}) out {} f_{};\n",
          index,
          ty.get_glsl_type(),
          name
        )
        .as_str();
      }

      fragment_shader += "void main() {\n";
      // Assignment of vertex outputs.
      for (ty, name) in shared_attributes.iter() {
        fragment_shader += format!("  {} {} = vs_{};\n", ty.get_glsl_type(), name, name).as_str();
      }
      // Get all removed nodes.
      let ignored = vertex_shader_traversed
        .iter()
        .copied()
        .filter(|n| *n != self.get_bootstrap_end_node())
        .collect::<Vec<_>>();
      // Build code.
      fragment_shader += self
        .build_function_code(&fragment_shader_traversed, &ignored, &list, true)
        .as_str();
      fragment_shader += "}\n";
    }

    (vertex_shader, fragment_shader)
  }

  pub fn get_bootstrap_node(&self) -> ArenaIndex {
    self.node_vertex.unwrap()
  }

  pub fn get_bootstrap_end_node(&self) -> ArenaIndex {
    self.node_output.unwrap()
  }
}

/// A graph that contains program logic.
pub struct ShaderGraph {
  nodes: Arena<ShaderGraphNode>,
  entities: Arena<ShaderGraphEntity>,
}
impl ShaderGraph {
  pub fn new() -> Self {
    Self {
      nodes: Arena::with_capacity(128),
      entities: Arena::with_capacity(256),
    }
  }

  pub fn add_node(&mut self, node: ShaderGraphNode) -> ArenaIndex {
    let id = self.nodes.insert(node);
    self.nodes.get_mut(id).unwrap().self_index = Some(id);
    id
  }

  pub fn add_entity(&mut self, entity: ShaderGraphEntity) -> ArenaIndex {
    let id = self.entities.insert(entity);
    self.entities.get_mut(id).unwrap().self_index = Some(id);
    id
  }

  pub fn add_node_builder(&mut self, builder: ShaderGraphNodeBuilder) -> ArenaIndex {
    builder.build(self)
  }

  pub fn connect_name(
    &mut self,
    source_node: ArenaIndex,
    source_name: &str,
    destination_node: ArenaIndex,
    destination_name: &str,
  ) -> Result<(), GraphBuildingError> {
    let source = {
      let node = self
        .nodes
        .get(source_node)
        .ok_or(GraphBuildingError::UnknownSource)?;
      let entity = node
        .outputs
        .iter()
        .find(|i| self.entities.get(**i).unwrap().name == source_name)
        .ok_or(GraphBuildingError::UnknownSource)?;
      *entity
    };
    let destination = {
      let node = self
        .nodes
        .get(destination_node)
        .ok_or(GraphBuildingError::UnknownDestination)?;
      let entity = node
        .inputs
        .iter()
        .find(|i| self.entities.get(**i).unwrap().name == destination_name)
        .ok_or(GraphBuildingError::UnknownDestination)?;
      *entity
    };

    self.connect(source, destination)
  }

  pub fn connect(
    &mut self,
    source: ArenaIndex,
    destination: ArenaIndex,
  ) -> Result<(), GraphBuildingError> {
    // Connect source
    {
      let source_entity = self
        .entities
        .get_mut(source)
        .ok_or(GraphBuildingError::UnknownSource)?;
      source_entity.connection.push(destination);
    }
    // Connect destination
    {
      let destination_entity = self
        .entities
        .get_mut(destination)
        .ok_or(GraphBuildingError::UnknownDestination)?;
      destination_entity.connection.push(source);
    }

    Ok(())
  }

  fn traverse_backwards(&self, entity_index: ArenaIndex, output: &mut Vec<ArenaIndex>) {
    // Find node of current entity.
    let (parent_entity_index, node_index) = {
      let entity = self.entities.get(entity_index).unwrap();
      let node_index = entity.parent_node.unwrap();
      let parent_entity_index = entity.connection.first().copied().unwrap();
      (parent_entity_index, node_index)
    };

    // Find connection destination node
    let parent_node_index = {
      let entity = self.entities.get(parent_entity_index).unwrap();
      entity.parent_node.unwrap()
    };
    let parent_node = self.nodes.get(parent_node_index).unwrap();
    output.push(node_index);
    output.push(parent_node_index);

    // Traverse through all inputs.
    for input_index in &parent_node.inputs {
      self.traverse_backwards(*input_index, output);
    }
  }

  fn dedup_unordered(list: &[ArenaIndex]) -> Vec<ArenaIndex> {
    let mut deduped = Vec::new();
    for item in list {
      if deduped.contains(item) {
        continue;
      }
      deduped.push(*item);
    }
    deduped
  }

  pub fn traverse(&self, list: &[ArenaIndex]) -> Vec<ArenaIndex> {
    // Build traversing list
    let mut traversed_nodes = Vec::new();
    for index in list {
      self.traverse_backwards(*index, &mut traversed_nodes);
    }

    Self::dedup_unordered(&traversed_nodes)
  }

  fn traverse_subtree(
    &self,
    nodes_source: &[ArenaIndex],
    nodes_target: &[ArenaIndex],
    shared: &mut Vec<ArenaIndex>,
    current: ArenaIndex,
  ) {
    let inputs = self.nodes.get(current).unwrap().inputs.clone();
    for input in inputs {
      let con = {
        let entity = self.entities.get(input).unwrap();
        entity.connection.first().copied().unwrap()
      };
      let target = {
        let entity = self.entities.get(con).unwrap();
        entity.parent_node.unwrap()
      };

      if nodes_source.contains(&target) && nodes_target.contains(&target) {
        shared.push(target);
      } else {
        self.traverse_subtree(nodes_source, nodes_target, shared, target);
      }
    }
  }

  pub fn find_shared_subtrees(
    &self,
    nodes_source: &[ArenaIndex],
    nodes_target: &[ArenaIndex],
  ) -> Vec<ArenaIndex> {
    let mut shared = Vec::new();

    let entry_index = nodes_target.first().copied().unwrap();
    self.traverse_subtree(nodes_source, nodes_target, &mut shared, entry_index);

    Self::dedup_unordered(&shared)
  }
}

#[derive(Debug)]
pub struct ShaderGraphEntity {
  self_index: Option<ArenaIndex>,
  parent_node: Option<ArenaIndex>,
  connection: Vec<ArenaIndex>,
  name: String,
  ty: ShaderType,
}
impl ShaderGraphEntity {
  pub fn new(name: &str, ty: ShaderType) -> Self {
    Self {
      ty,
      name: name.to_string(),
      parent_node: None,
      self_index: None,
      connection: Vec::new(),
    }
  }
}

pub type GeneratorType = Box<dyn Fn(&[(usize, String)], &[(usize, String)]) -> String>;

pub struct ShaderGraphNode {
  self_index: Option<ArenaIndex>,
  inputs: Vec<ArenaIndex>,
  outputs: Vec<ArenaIndex>,
  generator: GeneratorType,
}

pub trait ShaderNodeGenerator {
  fn get_inputs(&self) -> Vec<ShaderGraphEntity>;
  fn get_outputs(&self) -> Vec<ShaderGraphEntity>;
  fn generate(&self, inputs: &[(usize, String)], outputs: &[(usize, String)]) -> String;
}

pub struct ShaderGraphNodeBuilder {
  inputs: Vec<ShaderGraphEntity>,
  outputs: Vec<ShaderGraphEntity>,
  generator: GeneratorType,
}
impl ShaderGraphNodeBuilder {
  pub fn new(f: GeneratorType) -> Self {
    Self {
      inputs: Vec::new(),
      outputs: Vec::new(),
      generator: f,
    }
  }

  pub fn from_generator<T: ShaderNodeGenerator + 'static>(generator: T) -> Self {
    Self {
      inputs: generator.get_inputs(),
      outputs: generator.get_outputs(),
      generator: Box::new(move |i, o| generator.generate(i, o)),
    }
  }

  pub fn add_input(mut self, name: &str, ty: ShaderType) -> Self {
    self.inputs.push(ShaderGraphEntity::new(name, ty));
    self
  }

  pub fn add_output(mut self, name: &str, ty: ShaderType) -> Self {
    self.outputs.push(ShaderGraphEntity::new(name, ty));
    self
  }

  fn build_entities(
    graph: &mut ShaderGraph,
    parent: ArenaIndex,
    entities: Vec<ShaderGraphEntity>,
  ) -> Vec<ArenaIndex> {
    let inputs = {
      entities
        .into_iter()
        .map(|i| graph.add_entity(i))
        .collect::<Vec<_>>()
    };
    inputs
      .into_iter()
      .map(|i| {
        let mut entity = graph.entities.get_mut(i).unwrap();
        entity.self_index = Some(i);
        entity.parent_node = Some(parent);
        i
      })
      .collect::<Vec<_>>()
  }

  #[allow(clippy::needless_collect)]
  pub fn build(self, graph: &mut ShaderGraph) -> ArenaIndex {
    // Create node.
    let node_index = graph.add_node(ShaderGraphNode {
      generator: self.generator,
      self_index: None,
      inputs: Vec::new(),
      outputs: Vec::new(),
    });

    // Assign itself
    {
      let mut node = graph.nodes.get_mut(node_index).unwrap();
      node.self_index = Some(node_index);
    }

    // Map entities
    let inputs = Self::build_entities(graph, node_index, self.inputs);
    let outputs = Self::build_entities(graph, node_index, self.outputs);
    let mut node = graph.nodes.get_mut(node_index).unwrap();
    node.inputs = inputs;
    node.outputs = outputs;

    node_index
  }
}

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

struct GraphBootstrap {
  attributes: Vec<VertexAttribute>,
}
impl ShaderNodeGenerator for GraphBootstrap {
  fn get_inputs(&self) -> Vec<ShaderGraphEntity> {
    Vec::new()
  }

  fn get_outputs(&self) -> Vec<ShaderGraphEntity> {
    self
      .attributes
      .iter()
      .map(|a| ShaderGraphEntity::new(a.name.as_str(), ShaderType::from(a.format)))
      .collect::<Vec<_>>()
  }

  fn generate(&self, _: &[(usize, String)], outputs: &[(usize, String)]) -> String {
    let mut output = String::with_capacity(1024);
    for (index, name) in outputs.iter() {
      output += format!(
        "{} {} = a_{};\n",
        ShaderType::from(self.attributes[*index].format).get_glsl_type(),
        name,
        self.attributes[*index].name
      )
      .as_str();
    }
    output
  }
}

struct GraphBootstrapEnd {
  outputs: Vec<(String, ShaderType)>,
}
impl ShaderNodeGenerator for GraphBootstrapEnd {
  fn get_inputs(&self) -> Vec<ShaderGraphEntity> {
    let mut output = Vec::with_capacity(self.outputs.len() + 1);
    output.push(ShaderGraphEntity::new("position", ShaderType::Float4));
    output.extend(
      self
        .outputs
        .iter()
        .map(|(name, ty)| ShaderGraphEntity::new(name.as_str(), *ty)),
    );
    output
  }

  fn get_outputs(&self) -> Vec<ShaderGraphEntity> {
    Vec::new()
  }

  fn generate(&self, inputs: &[(usize, String)], _: &[(usize, String)]) -> String {
    let mut output = String::with_capacity(1024);
    for (index, name) in inputs.iter() {
      if *index == 0 {
        output += format!("gl_Position = {};", name).as_str();
        continue;
      }

      output += format!("f_{} = {};\n", self.outputs[*index - 1].0, name).as_str();
    }
    output
  }
}

/// Describes a sized struct that is used as a vertex buffer.
pub trait VertexStruct: Sized {
  fn generate_raw_u8(slice: &[Self]) -> &[u8];
  fn generate_attributes() -> Vec<VertexAttribute>;
  fn generate_buffer() -> VertexBuffer;
}

#[derive(Error, Debug)]
pub enum GraphBuildingError {
  #[error("The source connection is unknown")]
  UnknownSource,
  #[error("The destination connection is unknown")]
  UnknownDestination,
}
