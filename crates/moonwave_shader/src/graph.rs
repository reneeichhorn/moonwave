use std::any::Any;
use std::any::TypeId;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::Hash;
use std::hash::Hasher;
use std::sync::Arc;

use generational_arena::Arena;
use moonwave_resources::{VertexAttribute, VertexBuffer};
use thiserror::Error;
use uuid::Uuid;

use crate::{ShaderType, UniformStruct, VertexStruct};

pub use generational_arena::Index;

const MAX_NODES: usize = 128;
const MAX_INPUT_OUTPUTS_PER_NODE: usize = 16;

pub struct ShaderGraph {
  vertex_attributes: Vec<VertexAttribute>,
  vertex_buffer: Option<VertexBuffer>,
  vertex_output_node: Option<Index>,
  color_outputs: Vec<(String, ShaderType, Index)>,
  uniforms: Vec<Uniform>,
  textures: Vec<Texture>,
  texture_arrays: Vec<TextureArray>,
  nodes: Arena<Node>,
}

impl ShaderGraph {
  pub fn new() -> Self {
    Self {
      vertex_buffer: None,
      nodes: Arena::with_capacity(MAX_NODES),
      color_outputs: Vec::new(),
      vertex_attributes: Vec::new(),
      uniforms: Vec::new(),
      textures: Vec::new(),
      texture_arrays: Vec::new(),
      vertex_output_node: None,
    }
  }

  pub fn get_color_outputs(&self) -> &Vec<(String, ShaderType, Index)> {
    &self.color_outputs
  }

  pub fn add_vertex_attributes<T: VertexStruct + 'static>(&mut self) -> (Index, Index) {
    self.vertex_attributes = T::generate_attributes();
    self.vertex_buffer = Some(T::generate_buffer());
    let index = self.add_node(VertexAttributesNode {
      attributes: self.vertex_attributes.clone(),
    });
    let output_node = self.add_node(VertexShaderOutputNode {});
    self.vertex_output_node = Some(output_node);
    (index, output_node)
  }

  pub fn add_vertex_output_only(&mut self) -> Index {
    let output_node = self.add_node(VertexShaderOutputNode {});
    self.vertex_output_node = Some(output_node);
    output_node
  }

  pub fn add_color_output(&mut self, name: &str, format: ShaderType) -> Index {
    let string = name.to_string();
    let index = self.add_node(ColorOutputNode {
      name: string.clone(),
    });
    self.color_outputs.push((string, format, index));
    index
  }

  pub fn add_uniform<T: UniformStruct>(&mut self, name: &str) -> (Uuid, Index) {
    let node = UniformNode {
      name: name.to_string(),
      attributes: T::generate_attributes(),
      dependencies: T::generate_dependencies(),
    };
    let index = self.add_node(node);
    let id = Uuid::new_v4();
    self.uniforms.push(Uniform {
      id,
      ty_id: T::get_id(),
      node_index: index,
      name: T::generate_name(),
      attributes: T::generate_attributes(),
    });
    (id, index)
  }

  pub fn add_sampled_texture(&mut self, name: &str) -> (Index, Uuid) {
    let id = Uuid::new_v4();
    let node = TextureNode {
      name: name.to_string(),
    };
    let node_index = self.add_node(node);
    self.textures.push(Texture {
      id,
      node_index,
      name: name.to_string(),
    });
    (node_index, id)
  }

  pub fn add_sampled_texture_array(&mut self, name: &str, size: u32) -> (Index, Uuid) {
    let id = Uuid::new_v4();
    let node = TextureArrayNode {
      name: name.to_string(),
    };
    let node_index = self.add_node(node);
    self.texture_arrays.push(TextureArray {
      id,
      node_index,
      name: name.to_string(),
      size,
    });
    (node_index, id)
  }

  /// Add a new node into the graph.
  pub fn add_node<T: ShaderNode>(&mut self, node: T) -> Index {
    self.nodes.insert(Node {
      node: Arc::new(node),
      inputs: [None; MAX_INPUT_OUTPUTS_PER_NODE],
    })
  }

  /// Adds another graph into thre current graph.
  pub fn add_sub_graph(
    &mut self,
    graph: &ShaderGraph,
    input_node: Option<Index>,
    output_node: Option<Index>,
  ) -> (Option<Index>, Option<Index>) {
    // Mapping for indices
    let mut mapped = HashMap::new();

    // Add the vertex output first as there is always only once in the graph!
    if let (Some(output), Some(output_current)) =
      (graph.vertex_output_node, self.vertex_output_node)
    {
      // Insert for mapping purpose.
      mapped.insert(output, output_current);
      // Override set inputs from sub graph.
      let old_node = graph.nodes.get(output).unwrap();
      let new_node = self.nodes.get_mut(output_current).unwrap();
      for (index, input) in old_node.inputs.iter().enumerate() {
        if let Some(input) = input {
          new_node.inputs[index] = Some(*input);
        }
      }
    }

    // Check uniform requirements
    for uniform in &graph.uniforms {
      let id = uniform.ty_id;
      if let Some(existing) = self.uniforms.iter().find(|u| u.ty_id == id) {
        // Uniform is already used in this graph therefore reuse that.
        mapped.insert(uniform.node_index, existing.node_index);
      } else {
        // New uniform that needs to be inserted to current graph.
        let old_node = graph.nodes.get(uniform.node_index).unwrap();
        let uniform_node = self.nodes.insert(old_node.clone());
        mapped.insert(uniform.node_index, uniform_node);
        let new_uniform = Uniform {
          node_index: uniform_node,
          ..uniform.clone()
        };
        self.uniforms.push(new_uniform);
      }
    }

    // Check textures
    for texture in &graph.textures {
      let name = &texture.name;
      if let Some(existing) = self.textures.iter().find(|u| &u.name == name) {
        // Texture is already used in this graph therefore reuse that.
        mapped.insert(texture.node_index, existing.node_index);
      } else {
        // New texture that needs to be inserted to current graph.
        let old_node = graph.nodes.get(texture.node_index).unwrap();
        let texture_node = self.nodes.insert(old_node.clone());
        mapped.insert(texture.node_index, texture_node);
        let new_texture = Texture {
          node_index: texture_node,
          ..texture.clone()
        };
        self.textures.push(new_texture);
      }
    }

    // Color outputs
    for output in &graph.color_outputs {
      if let Some(existing) = self.color_outputs.iter().find(|u| u.0 == output.0) {
        // Output is already used in this graph therefore reuse that.
        mapped.insert(output.2, existing.2);
        // Override set inputs from sub graph.
        let old_node = graph.nodes.get(output.2).unwrap();
        let new_node = self.nodes.get_mut(existing.2).unwrap();
        for (index, input) in old_node.inputs.iter().enumerate() {
          if let Some(input) = input {
            new_node.inputs[index] = Some(*input);
          }
        }
      } else {
        // New output that needs to be inserted to current graph.
        let old_node = graph.nodes.get(output.2).unwrap();
        let uniform_node = self.nodes.insert(old_node.clone());
        mapped.insert(output.2, uniform_node);
        self
          .color_outputs
          .push((output.0.clone(), output.1, uniform_node));
      }
    }

    // Insert all nodes
    for (old_index, node) in graph.nodes.iter() {
      if mapped.contains_key(&old_index) {
        continue;
      }
      let new_index = self.nodes.insert(node.clone());
      mapped.insert(old_index, new_index);
    }

    // Map references.
    for new in mapped.values() {
      let node = self.nodes.get_mut(*new).unwrap();
      for input in node.inputs.iter_mut().flatten() {
        input.owner_node_index = *mapped.get(&input.owner_node_index).unwrap();
      }
    }

    // Map inputs and outputs.
    (
      input_node.map(|index| *mapped.get(&index).unwrap()),
      output_node.map(|index| *mapped.get(&index).unwrap()),
    )
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

  pub fn build(&mut self, outputs: &[Index], params: &ShaderBuildParams) -> BuiltShaderGraph {
    // Do some post processing on graph.
    for i in outputs {
      self.cleanup_passthrough(*i);
    }
    self.cleanup_passthrough(self.vertex_output_node.unwrap());

    let mut vertex_shader_code = String::with_capacity(1024 * 1024);
    let mut fragment_shader_code = String::with_capacity(1024 * 1024);

    // Traverse from the starting point of the vertex output node.
    let traversed_vertex_shader = {
      optick::event!("ShaderGraph::traverse_vertex_shader");
      let mut nodes = Vec::with_capacity(MAX_NODES);
      self.traverse(self.vertex_output_node.unwrap(), &mut nodes, params);
      Self::dedup_unordered(&nodes)
    };

    // Traverse from the starting point of each color output.
    let mut traversed_fragment_shader = {
      optick::event!("ShaderGraph::traverse_fragment_shader");
      let mut nodes = Vec::with_capacity(MAX_NODES);
      for node in outputs {
        self.traverse(*node, &mut nodes, params);
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
        // Same for textures
        if self.textures.iter().any(|t| t.node_index == *node_index) {
          continue;
        }
        // Same for textures arrays
        if self
          .texture_arrays
          .iter()
          .any(|t| t.node_index == *node_index)
        {
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
      .filter_map(|uniform| {
        let in_vs = traversed_vertex_shader.contains(&uniform.node_index);
        let in_fs = traversed_fragment_shader.contains(&uniform.node_index);
        if !in_vs && !in_fs {
          return None;
        }
        Some((in_vs, in_fs, uniform))
      })
      .enumerate()
      .map(|(index, (in_vs, in_fs, uniform))| BuiltUniform {
        id: uniform.id,
        ty_id: uniform.ty_id,
        binding: index,
        name: uniform.name.clone(),
        attributes: uniform.attributes.clone(),
        in_vs,
        in_fs,
      })
      .collect::<Vec<_>>();

    // Texture
    let textures = self
      .textures
      .iter()
      .filter_map(|texture| {
        let in_vs = traversed_vertex_shader.contains(&texture.node_index);
        let in_fs = traversed_fragment_shader.contains(&texture.node_index);
        if !in_vs && !in_fs {
          return None;
        }
        Some((in_vs, in_fs, texture))
      })
      .enumerate()
      .map(|(index, (in_vs, in_fs, texture))| BuiltTexture {
        name: texture.name.clone(),
        id: texture.id,
        binding: uniforms.len() + index,
        in_vs,
        in_fs,
      })
      .collect::<Vec<_>>();

    // Texture
    let texture_arrays = self
      .texture_arrays
      .iter()
      .filter_map(|texture| {
        let in_vs = traversed_vertex_shader.contains(&texture.node_index);
        let in_fs = traversed_fragment_shader.contains(&texture.node_index);
        if !in_vs && !in_fs {
          return None;
        }
        Some((in_vs, in_fs, texture))
      })
      .enumerate()
      .map(|(index, (in_vs, in_fs, texture))| BuiltTextureArray {
        name: texture.name.clone(),
        id: texture.id,
        size: texture.size,
        binding: uniforms.len() + textures.len() + index,
        in_vs,
        in_fs,
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
      let mut global_code = String::with_capacity(1024);
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

      // Code
      let mut function_code = String::with_capacity(1024);
      function_code += "void main() {\n";
      self.generate_code(
        &mut function_code,
        &mut global_code,
        &traversed_vertex_shader,
        &[],
        &params,
      );
      for (_, name) in &shared_attributes {
        function_code += format!("vs_{} = {};\n", name, name).as_str();
      }
      function_code += "}\n";
      vertex_shader_code += global_code.as_str();

      // Uniforms
      for uniform in &uniforms {
        if !uniform.in_vs {
          continue;
        }
        Self::generate_uniform(uniform, &mut vertex_shader_code);
      }

      vertex_shader_code += function_code.as_str();
    }

    // Fragment shader
    {
      let mut global_code = String::with_capacity(1024);
      optick::event!("ShaderGraph::generate_fragment_shader");
      fragment_shader_code += "#version 450\n\n";

      // Shared attributes for fragment shader.
      for (index, (ty, name)) in shared_attributes.iter().enumerate() {
        let is_flat = matches!(
          ty,
          ShaderType::UInt | ShaderType::UInt2 | ShaderType::UInt3 | ShaderType::UInt4
        );

        fragment_shader_code += format!(
          "layout (location = {}) {} in {} vs_{};\n",
          index,
          if is_flat { "flat" } else { "" },
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

      // Code
      let mut function_code = String::with_capacity(1024);
      function_code += "void main() {\n";
      for (ty, name) in &shared_attributes {
        function_code += format!("{} {} = vs_{};\n", ty.get_glsl_type(), name, name).as_str();
      }
      let vs = traversed_vertex_shader
        .iter()
        .copied()
        .filter(|it| self.uniforms.iter().find(|u| &u.node_index == it).is_none())
        .collect::<Vec<_>>();
      self.generate_code(
        &mut function_code,
        &mut global_code,
        &traversed_fragment_shader,
        &vs,
        &params,
      );
      function_code += "}\n";

      // Uniforms
      for texture in &textures {
        if !texture.in_fs {
          continue;
        }
        Self::generate_texture(texture, &mut fragment_shader_code);
      }
      for texture in &texture_arrays {
        if !texture.in_fs {
          continue;
        }
        Self::generate_texture_array(texture, &mut fragment_shader_code);
      }
      fragment_shader_code += global_code.as_str();

      for uniform in &uniforms {
        if !uniform.in_fs {
          continue;
        }
        Self::generate_uniform(uniform, &mut fragment_shader_code);
      }

      fragment_shader_code += function_code.as_str();
    }

    // Bind groups
    let mut bind_groups = Vec::new();
    for uniform in uniforms {
      bind_groups.push(BuiltShaderBindGroup::Uniform(uniform));
    }
    for texture in textures {
      bind_groups.push(BuiltShaderBindGroup::SampledTexture(texture));
    }
    for texture in texture_arrays {
      bind_groups.push(BuiltShaderBindGroup::SampledTextureArray(texture));
    }

    BuiltShaderGraph {
      vb: self.vertex_buffer.clone().unwrap(),
      vs: vertex_shader_code,
      fs: fragment_shader_code,
      bind_groups,
    }
  }

  fn generate_uniform(uniform: &BuiltUniform, output: &mut String) {
    *output += format!(
      "layout (set = {}, binding = 0) uniform {}_block {{\n",
      uniform.binding, uniform.name
    )
    .as_str();

    for attr in &uniform.attributes {
      *output += format!(
        "\t{} {};\n",
        attr.1.get_glsl_type(),
        attr.1.get_glsl_var(&attr.0)
      )
      .as_str();
    }

    *output += format!("}} {};\n", uniform.name).as_str();
  }

  fn generate_texture(texture: &BuiltTexture, output: &mut String) {
    *output += format!(
      "layout (set = {}, binding = 0) uniform texture2D t_{};\n",
      texture.binding, texture.name
    )
    .as_str();
    *output += format!(
      "layout (set = {}, binding = 1) uniform sampler s_{};\n",
      texture.binding, texture.name
    )
    .as_str();
  }

  fn generate_texture_array(texture: &BuiltTextureArray, output: &mut String) {
    *output += format!(
      "layout (set = {}, binding = 0) uniform texture2D t_{}[{}];\n",
      texture.binding, texture.name, texture.size
    )
    .as_str();
    *output += format!(
      "layout (set = {}, binding = 1) uniform sampler s_{};\n",
      texture.binding, texture.name
    )
    .as_str();
  }

  fn generate_code(
    &self,
    output: &mut String,
    global: &mut String,
    nodes: &[Index],
    skipped: &[Index],
    params: &ShaderBuildParams,
  ) {
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

      node.node.generate_global_code(&inputs, &outputs, global);
      node
        .node
        .generate_with_params(&inputs, &outputs, output, params);
    }
  }

  fn dedup_unordered(list: &[Index]) -> Vec<Index> {
    let mut deduped = Vec::new();
    for item in list.iter().rev() {
      if deduped.contains(item) {
        continue;
      }
      deduped.push(*item);
    }
    deduped.reverse();
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

  fn traverse(&self, index: Index, output: &mut Vec<Index>, params: &ShaderBuildParams) {
    let node = self.nodes.get(index).unwrap();
    output.push(index);

    for (index, input) in node.inputs.iter().enumerate() {
      if let Some(input) = input {
        if !node.node.optimize_input(index, params) {
          // Node has beem optimized out with runtime expressions and we therefore don't care about the nodes here.
          continue;
        }

        self.traverse(input.owner_node_index, output, params);
      }
    }
  }

  fn cleanup_passthrough(&mut self, index: Index) {
    // List of changes required for the current node.
    let mut changes = Vec::new();

    // Go through inputs to find required changes.
    let node = self.nodes.get(index).unwrap().clone();
    for (index, input) in node.inputs.iter().enumerate() {
      if let Some(input) = input {
        // Is target node a passthrough
        let target_node = self.nodes.get(input.owner_node_index).unwrap();
        if let Some(_) = target_node.node.as_passthrough() {
          changes.push((
            index,
            target_node.inputs[input.owner_node_output].clone().unwrap(),
          ));
        }

        self.cleanup_passthrough(input.owner_node_index);
      }
    }

    // Apply changes mutably
    let node = self.nodes.get_mut(index).unwrap();
    for (index, new_input) in changes {
      node.inputs[index] = Some(new_input);
    }
  }
}

#[derive(Clone, Debug)]
struct Node {
  inputs: [Option<Input>; MAX_INPUT_OUTPUTS_PER_NODE],
  node: Arc<dyn ShaderNode>,
}
#[derive(Copy, Clone, Debug)]
struct Input {
  owner_node_index: Index,
  owner_node_output: usize,
}

#[derive(Clone)]
struct Uniform {
  id: Uuid,
  ty_id: Uuid,
  node_index: Index,
  name: String,
  attributes: Vec<(String, ShaderType)>,
}

#[derive(Clone)]
struct Texture {
  id: Uuid,
  name: String,
  node_index: Index,
}

#[derive(Clone)]
struct TextureArray {
  id: Uuid,
  name: String,
  size: u32,
  node_index: Index,
}

pub struct ShaderBuildParams {
  params: HashMap<std::any::TypeId, Box<dyn Any>>,
  pub hash: u64,
}

impl ShaderBuildParams {
  pub fn new() -> Self {
    Self {
      params: HashMap::new(),
      hash: 0,
    }
  }

  pub fn add<T: Any + Hash>(&mut self, value: T) {
    let mut hasher = DefaultHasher::default();
    hasher.write_u64(self.hash);
    value.hash(&mut hasher);
    self.hash = hasher.finish();

    self.params.insert(value.type_id(), Box::new(value));
  }

  pub fn get<T: Any>(&self) -> &T {
    self
      .params
      .get(&TypeId::of::<T>())
      .unwrap()
      .downcast_ref()
      .unwrap()
  }
}

pub trait ShaderNode: std::fmt::Debug + Send + Sync + 'static {
  /// Evaluates if the input is really used by this node.
  fn optimize_input(&self, _index: usize, _params: &ShaderBuildParams) -> bool {
    true
  }

  fn get_available_stages(&self) -> (bool, bool) {
    (true, true)
  }

  fn get_type_expectation(&self, _index: usize) -> Option<ShaderType> {
    None
  }

  fn get_outputs(&self) -> Vec<ShaderType> {
    Vec::new()
  }

  fn as_passthrough(&self) -> Option<&InputPassthroughNode> {
    None
  }

  fn generate_with_params(
    &self,
    inputs: &[Option<String>],
    outputs: &[Option<String>],
    output: &mut String,
    _params: &ShaderBuildParams,
  ) {
    self.generate(inputs, outputs, output)
  }

  fn generate(
    &self,
    _inputs: &[Option<String>],
    _outputs: &[Option<String>],
    _output: &mut String,
  ) {
  }

  fn generate_global_code(
    &self,
    _inputs: &[Option<String>],
    _outputs: &[Option<String>],
    _output: &mut String,
  ) {
  }
}

#[derive(Clone, Debug)]
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

#[derive(Debug)]
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

#[derive(Debug)]
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

#[derive(Debug)]
struct UniformNode {
  name: String,
  attributes: Vec<(String, ShaderType)>,
  dependencies: Vec<(String, Vec<(String, ShaderType)>)>,
}

impl ShaderNode for UniformNode {
  fn get_outputs(&self) -> Vec<ShaderType> {
    self
      .attributes
      .iter()
      .map(|(_, ty)| *ty)
      .collect::<Vec<_>>()
  }

  fn generate_global_code(
    &self,
    _inputs: &[Option<String>],
    _outputs: &[Option<String>],
    output: &mut String,
  ) {
    for (name, attributes) in &self.dependencies {
      *output += format!("struct {} {{\n", name).as_str();
      for (attr_name, ty) in attributes {
        *output += format!("{} {};\n", ty.get_glsl_type(), ty.get_glsl_var(&attr_name)).as_str();
      }
      *output += "};\n\n";
    }
  }

  fn generate(&self, _inputs: &[Option<String>], outputs: &[Option<String>], output: &mut String) {
    for (index, (name, ty)) in self.attributes.iter().enumerate() {
      *output += format!(
        "{} {} = {}_uniform.{};\n",
        ty.get_glsl_type(),
        ty.get_glsl_var(outputs[index].as_ref().unwrap()),
        self.name,
        name,
      )
      .as_str();
    }
  }
}

#[derive(Debug)]
struct TextureNode {
  name: String,
}
impl ShaderNode for TextureNode {
  fn get_outputs(&self) -> Vec<ShaderType> {
    vec![ShaderType::Float]
  }

  fn generate(
    &self,
    _inputs: &[Option<String>],
    _outputs: &[Option<String>],
    _output: &mut String,
  ) {
  }

  fn generate_global_code(
    &self,
    _inputs: &[Option<String>],
    outputs: &[Option<String>],
    output: &mut String,
  ) {
    *output += format!(
      r#"
      vec4 sample_fn_{}(vec2 uv) {{
        return texture(sampler2D(t_{}, s_{}), uv);
      }}
      "#,
      outputs[0].as_ref().unwrap(),
      &self.name,
      &self.name,
    )
    .as_str();
  }
}

#[derive(Debug)]
pub struct TextureSampleNode;

impl TextureSampleNode {
  pub const INPUT_TEXTURE: usize = 0;
  pub const INPUT_UV: usize = 1;
  pub const OUTPUT_COLOR: usize = 0;

  pub fn new() -> Self {
    Self
  }
}

impl ShaderNode for TextureSampleNode {
  fn generate(&self, inputs: &[Option<String>], outputs: &[Option<String>], output: &mut String) {
    *output += format!(
      "vec4 {} = sample_fn_{}({});\n",
      outputs[Self::OUTPUT_COLOR].as_ref().unwrap(),
      inputs[Self::INPUT_TEXTURE].as_ref().unwrap(),
      inputs[Self::INPUT_UV].as_ref().unwrap()
    )
    .as_str();
  }
}

#[derive(Debug)]
struct TextureArrayNode {
  name: String,
}
impl ShaderNode for TextureArrayNode {
  fn get_outputs(&self) -> Vec<ShaderType> {
    vec![ShaderType::Float]
  }

  fn generate(
    &self,
    _inputs: &[Option<String>],
    _outputs: &[Option<String>],
    _output: &mut String,
  ) {
  }

  fn generate_global_code(
    &self,
    _inputs: &[Option<String>],
    outputs: &[Option<String>],
    output: &mut String,
  ) {
    *output += format!(
      r#"
      vec4 sample_fn_arr_{}(vec2 uv, uint index) {{
        return texture(sampler2D(t_{}[index], s_{}), uv);
      }}
      "#,
      outputs[0].as_ref().unwrap(),
      &self.name,
      &self.name,
    )
    .as_str();
  }
}

#[derive(Debug)]
pub struct TextureArraySampleNode;

impl TextureArraySampleNode {
  pub const INPUT_TEXTURE: usize = 0;
  pub const INPUT_UV: usize = 1;
  pub const INPUT_INDEX: usize = 2;
  pub const OUTPUT_COLOR: usize = 0;

  pub fn new() -> Self {
    Self
  }
}

impl ShaderNode for TextureArraySampleNode {
  fn generate(&self, inputs: &[Option<String>], outputs: &[Option<String>], output: &mut String) {
    *output += format!(
      "vec4 {} = sample_fn_arr_{}({}, {});\n",
      outputs[Self::OUTPUT_COLOR].as_ref().unwrap(),
      inputs[Self::INPUT_TEXTURE].as_ref().unwrap(),
      inputs[Self::INPUT_UV].as_ref().unwrap(),
      inputs[Self::INPUT_INDEX].as_ref().unwrap()
    )
    .as_str();
  }
}

#[derive(Debug)]
pub struct InputPassthroughNode {
  inputs: Vec<(ShaderType, String)>,
}

impl InputPassthroughNode {
  pub fn new() -> Self {
    Self { inputs: Vec::new() }
  }

  pub fn add_input(mut self, ty: ShaderType, default: &str) -> Self {
    self.inputs.push((ty, default.to_string()));
    self
  }
}

impl ShaderNode for InputPassthroughNode {
  fn as_passthrough(&self) -> Option<&Self> {
    Some(self)
  }

  fn generate(
    &self,
    _inputs: &[Option<String>],
    _outputs: &[Option<String>],
    _output: &mut String,
  ) {
    panic!("Passthrough node must be elimated in graph pre-processor");
  }
}

#[derive(Debug)]
pub struct BuiltUniform {
  pub binding: usize,
  pub ty_id: Uuid,
  pub id: Uuid,
  name: String,
  attributes: Vec<(String, ShaderType)>,
  pub in_vs: bool,
  pub in_fs: bool,
}

#[derive(Debug)]
pub struct BuiltTexture {
  pub name: String,
  pub binding: usize,
  pub id: Uuid,
  pub in_vs: bool,
  pub in_fs: bool,
}

#[derive(Debug)]
pub struct BuiltTextureArray {
  pub name: String,
  pub size: u32,
  pub binding: usize,
  pub id: Uuid,
  pub in_vs: bool,
  pub in_fs: bool,
}

#[derive(Debug)]
pub struct BuiltShaderGraph {
  pub vb: VertexBuffer,
  pub vs: String,
  pub fs: String,
  pub bind_groups: Vec<BuiltShaderBindGroup>,
}

#[derive(Debug)]
pub enum BuiltShaderBindGroup {
  SampledTexture(BuiltTexture),
  SampledTextureArray(BuiltTextureArray),
  Uniform(BuiltUniform),
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
