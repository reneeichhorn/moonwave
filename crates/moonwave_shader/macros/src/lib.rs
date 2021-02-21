use heck::{ShoutySnakeCase, SnakeCase};
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{parse_macro_input, GenericArgument, ItemStruct, Path, PathArguments, Type};

fn path_to_string(path: &Path) -> String {
  path
    .segments
    .iter()
    .map(|segment| {
      let postfix = match &segment.arguments {
        PathArguments::AngleBracketed(x) => match x.args.first().as_ref().unwrap() {
          GenericArgument::Type(Type::Path(path)) => format!("<{}>", path_to_string(&path.path)),
          _ => "".to_string(),
        },
        _ => "".to_string(),
      };
      format!("{}{}", segment.ident.to_string(), postfix)
    })
    .collect::<Vec<_>>()
    .join("::")
}

struct GlslType {
  enum_type: String,
  glsl_type: String,
  size: usize,
}

fn path_to_glsl_type(ty: &Type) -> Option<GlslType> {
  match ty {
    Type::Path(path) => {
      let full_path = path_to_string(&path.path);
      match full_path.as_str() {
        "Matrix4<f32>" => Some(GlslType {
          enum_type: "Matrix4".to_string(),
          glsl_type: "mat4".to_string(),
          size: 4 * 4,
        }),
        "Vector4<f32>" => Some(GlslType {
          enum_type: "Float4".to_string(),
          glsl_type: "vec4".to_string(),
          size: 4 * 4,
        }),
        "Vector3<f32>" => Some(GlslType {
          enum_type: "Float3".to_string(),
          glsl_type: "vec3".to_string(),
          size: 4 * 3,
        }),
        "Vector2<f32>" => Some(GlslType {
          enum_type: "Float2".to_string(),
          glsl_type: "vec2".to_string(),
          size: 4 * 2,
        }),
        "f32" => Some(GlslType {
          enum_type: "Float".to_string(),
          glsl_type: "float".to_string(),
          size: 4,
        }),
        "Vector4<u32>" => Some(GlslType {
          enum_type: "UInt4".to_string(),
          glsl_type: "uvec4".to_string(),
          size: 4 * 4,
        }),
        "Vector3<u32>" => Some(GlslType {
          enum_type: "UInt3".to_string(),
          glsl_type: "uvec3".to_string(),
          size: 4 * 3,
        }),
        "Vector2<u32>" => Some(GlslType {
          enum_type: "UInt2".to_string(),
          glsl_type: "uvec2".to_string(),
          size: 4 * 2,
        }),
        "u32" => Some(GlslType {
          enum_type: "UInt".to_string(),
          glsl_type: "u32".to_string(),
          size: 4,
        }),
        _ => None,
      }
    }
    _ => None,
  }
}

#[proc_macro_attribute]
pub fn vertex(_attr: TokenStream, item: TokenStream) -> TokenStream {
  // Parse basic structure.
  let item = parse_macro_input!(item as ItemStruct);
  let struct_ident = item.ident.clone();

  // Structure attribute parsing
  let mut offset = 0;
  let mut attribute_descs = Vec::with_capacity(item.fields.len());
  let mut shader_outputs = Vec::with_capacity(item.fields.len());
  let mut shader_outputs_constants = Vec::with_capacity(item.fields.len());

  for (index, attr) in item.fields.iter().enumerate() {
    let name = attr
      .ident
      .clone()
      .unwrap_or_else(|| panic!("All vertex struct fields must be named"));
    let name_str = name.to_string();

    let ty = path_to_glsl_type(&attr.ty)
      .unwrap_or_else(|| panic!("Unknown types can't be used within a vertex struct"));

    // Attribute desc
    let attribute_ty = format_ident!("{}", ty.enum_type);
    attribute_descs.push(quote! {
      moonwave_resources::VertexAttribute {
        name: #name_str.to_string(),
        offset: #offset as u64,
        location: #index,
        format: moonwave_resources::VertexAttributeFormat::#attribute_ty,
      }
    });

    // Shader nodes
    shader_outputs.push(quote! {
      moonwave_shader::ShaderNamedEntity::new(#name_str, moonwave_shader::ShaderEntity::Variable(moonwave_shader::ShaderEntityType::#attribute_ty))
    });

    // Constants
    let snaked = format_ident!("OUTPUT_{}", name_str.to_shouty_snake_case());
    shader_outputs_constants.push(quote! {
      pub const #snaked: usize = #index;
    });

    offset += ty.size;
  }

  // Build new content
  TokenStream::from(quote! {
    #[repr(C)]
    #[derive(Copy, Clone, Debug)]
    #item

    impl #struct_ident {
      #(#shader_outputs_constants)*
    }

    unsafe impl moonwave_common::bytemuck::Pod for #struct_ident {}
    unsafe impl moonwave_common::bytemuck::Zeroable for #struct_ident {}

    impl moonwave_shader::VertexStruct for #struct_ident {
      fn generate_raw_u8(slice: &[Self]) -> &[u8] {
        moonwave_common::bytemuck::cast_slice(slice)
      }

      fn generate_attributes() -> Vec<moonwave_resources::VertexAttribute> {
        vec![
          #(#attribute_descs),*
        ]
      }

      fn generate_buffer() -> moonwave_resources::VertexBuffer {
        moonwave_resources::VertexBuffer {
          stride: #offset as u64,
          attributes: Self::generate_attributes(),
        }
      }
    }
  })
}

#[proc_macro_attribute]
pub fn uniform(_attr: TokenStream, item: TokenStream) -> TokenStream {
  // Parse basic structure.
  let item = parse_macro_input!(item as ItemStruct);
  let struct_ident = item.ident.clone();
  let struct_name_snakecase = item.ident.to_string().to_snake_case();

  // Structure attribute parsing
  let mut attribute_descs = Vec::with_capacity(item.fields.len());
  let mut shader_outputs_constants = Vec::with_capacity(item.fields.len());

  for (index, attr) in item.fields.iter().enumerate() {
    let name = attr
      .ident
      .clone()
      .unwrap_or_else(|| panic!("All vertex struct fields must be named"));
    let name_str = name.to_string();

    let ty = path_to_glsl_type(&attr.ty)
      .unwrap_or_else(|| panic!("Unknown types can't be used within a vertex struct"));

    // Attribute desc
    let attribute_ty = format_ident!("{}", ty.enum_type);
    attribute_descs.push(quote! {
      (#name_str.to_string(), moonwave_shader::ShaderType::#attribute_ty)
    });

    // Constants
    let snaked = format_ident!("OUTPUT_{}", name_str.to_shouty_snake_case());
    shader_outputs_constants.push(quote! {
      pub const #snaked: usize = #index;
    });
  }

  TokenStream::from(quote! {
    #[repr(C)]
    #[derive(Copy, Clone, Debug, moonwave_shader::std140::AsStd140)]
    #item

    impl #struct_ident {
      #(#shader_outputs_constants)*
    }

    impl moonwave_shader::UniformStruct for #struct_ident {
      fn generate_raw_u8(&self) -> Vec<u8> {
        use moonwave_shader::{AsStd140, Std140};
        self.as_std140().as_bytes().to_vec()
      }

      fn generate_attributes() -> Vec<(String, moonwave_shader::ShaderType)> {
        vec![
          #(#attribute_descs),*
        ]
      }

      fn generate_name() -> String {
        #struct_name_snakecase.to_string()
      }
    }

    impl moonwave_core::BindGroupLayoutSingleton for #struct_ident {
      fn get_bind_group_lazy(core: &Core) -> moonwave_resources::ResourceRc<moonwave_resources::BindGroupLayout> {
        static cell: moonwave_core::OnceCell<moonwave_resources::ResourceRc<moonwave_resources::BindGroupLayout>> = moonwave_core::OnceCell::new();
        cell.get_or_init(|| {
          let desc = moonwave_resources::BindGroupLayoutDescriptor::new()
            .add_entry(0, moonwave_resources::BindGroupLayoutEntryType::UniformBuffer);
          let layout = moonwave_core::block_on(core.create_bind_group_layout(desc));
          layout
        }).clone()
      }
    }
  })
}
