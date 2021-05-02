use heck::{ShoutySnakeCase, SnakeCase};
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{parse_macro_input, Expr, GenericArgument, ItemStruct, Path, PathArguments, Type};
use uuid::Uuid;

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

#[derive(Default)]
struct GlslType {
  enum_type: String,
  glsl_type: String,
  size: usize,
  array_len: Option<Expr>,
  array_ty: Option<Box<GlslType>>,
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
          ..Default::default()
        }),
        "Vector4<f32>" => Some(GlslType {
          enum_type: "Float4".to_string(),
          glsl_type: "vec4".to_string(),
          size: 4 * 4,
          ..Default::default()
        }),
        "Vector3<f32>" => Some(GlslType {
          enum_type: "Float3".to_string(),
          glsl_type: "vec3".to_string(),
          size: 4 * 3,
          ..Default::default()
        }),
        "Vector2<f32>" => Some(GlslType {
          enum_type: "Float2".to_string(),
          glsl_type: "vec2".to_string(),
          size: 4 * 2,
          ..Default::default()
        }),
        "f32" => Some(GlslType {
          enum_type: "Float".to_string(),
          glsl_type: "float".to_string(),
          size: 4,
          ..Default::default()
        }),
        "Vector4<u32>" => Some(GlslType {
          enum_type: "UInt4".to_string(),
          glsl_type: "uvec4".to_string(),
          size: 4 * 4,
          ..Default::default()
        }),
        "Vector3<u32>" => Some(GlslType {
          enum_type: "UInt3".to_string(),
          glsl_type: "uvec3".to_string(),
          size: 4 * 3,
          ..Default::default()
        }),
        "Vector2<u32>" => Some(GlslType {
          enum_type: "UInt2".to_string(),
          glsl_type: "uvec2".to_string(),
          size: 4 * 2,
          ..Default::default()
        }),
        "u32" => Some(GlslType {
          enum_type: "UInt".to_string(),
          glsl_type: "u32".to_string(),
          size: 4,
          ..Default::default()
        }),
        _ => Some(GlslType {
          enum_type: "Struct".to_string(),
          glsl_type: full_path.clone(),
          size: 0,
          ..Default::default()
        }),
      }
    }
    Type::Array(arr) => {
      let ty = path_to_glsl_type(&*arr.elem).unwrap();

      Some(GlslType {
        enum_type: "Array".to_string(),
        glsl_type: format!("{}[]", ty.glsl_type),
        size: 0,
        array_len: Some(arr.len.clone()),
        array_ty: Some(Box::new(ty)),
      })
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
  let mut has_uvs = false;
  let mut has_normal = false;
  let mut has_tangent = false;
  let mut has_bitangent = false;

  for (index, attr) in item.fields.iter().enumerate() {
    let name = attr
      .ident
      .clone()
      .unwrap_or_else(|| panic!("All vertex struct fields must be named"));
    let name_str = name.to_string();

    match name_str.as_str() {
      "uv" => has_uvs = true,
      "normal" => has_normal = true,
      "tangent" => has_tangent = true,
      "bitangent" => has_bitangent = true,
      _ => {}
    }

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

  // Has uv support
  let uv_support = if has_uvs {
    quote! {
      impl moonwave_scene::MeshVertexUV for #struct_ident {
        fn get_uv(&self) -> &Vector2<f32> {
          &self.uv
        }
        fn get_uv_mut(&mut self) -> &mut Vector2<f32> {
          &mut self.uv
        }
      }
    }
  } else {
    TokenStream2::new()
  };

  // Has normal support
  let normal_support = if has_normal && has_tangent && has_bitangent {
    quote! {
      impl moonwave_scene::MeshVertexNormal for #struct_ident {
        fn get_normal(&self) -> &Vector3<f32> {
          &self.normal
        }
        fn get_normal_mut(&mut self) -> &mut Vector3<f32> {
          &mut self.normal
        }

        fn get_tangent(&self) -> &Vector3<f32> {
          &self.tangent
        }
        fn get_tangent_mut(&mut self) -> &mut Vector3<f32> {
          &mut self.tangent
        }

        fn get_bitangent(&self) -> &Vector3<f32> {
          &self.bitangent
        }
        fn get_bitangent_mut(&mut self) -> &mut Vector3<f32> {
          &mut self.bitangent
        }
      }
    }
  } else {
    TokenStream2::new()
  };

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

    impl moonwave_scene::MeshVertex for #struct_ident {
      fn get_position(&self) -> &Vector3<f32> {
        &self.position
      }
      fn get_position_mut(&mut self) -> &mut Vector3<f32> {
        &mut self.position
      }
    }

    #uv_support
    #normal_support
  })
}

fn struct_copy(vec: &mut Vec<TokenStream2>, ty: &GlslType) {
  match ty.enum_type.as_str() {
    "Struct" => {
      let name = ty.glsl_type.clone();
      let ident = format_ident!("{}", name);
      vec.push(quote! { (#name.to_string(), #ident::generate_attributes()) });
    }
    "Array" => {
      struct_copy(vec, &*ty.array_ty.as_ref().unwrap());
    }
    _ => {}
  }
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
  let mut struct_dependencies = Vec::new();

  for (index, attr) in item.fields.iter().enumerate() {
    let name = attr
      .ident
      .clone()
      .unwrap_or_else(|| panic!("All vertex struct fields must be named"));
    let name_str = name.to_string();

    let ty = path_to_glsl_type(&attr.ty)
      .unwrap_or_else(|| panic!("Unknown types can't be used within a vertex struct"));

    // Attribute desc
    struct_copy(&mut struct_dependencies, &ty);
    let attribute_ty = match ty.enum_type.as_str() {
      "Struct" => {
        let name = ty.glsl_type;
        quote! { Struct(#name) }
      }
      "Array" => {
        let len = ty.array_len.as_ref().unwrap();
        let ty = ty.array_ty.as_ref().unwrap();
        let name = &ty.glsl_type;
        quote! {
          Array(#name, #len)
        }
      }
      _ => {
        let ident = format_ident!("{}", ty.enum_type);
        quote! { #ident }
      }
    };

    attribute_descs.push(quote! {
      (#name_str.to_string(), moonwave_shader::ShaderType::#attribute_ty)
    });

    // Constants
    let snaked = format_ident!("OUTPUT_{}", name_str.to_shouty_snake_case());
    shader_outputs_constants.push(quote! {
      pub const #snaked: usize = #index;
    });
  }

  let uuid = Uuid::new_v4().to_u128_le();

  TokenStream::from(quote! {
    #[repr(C)]
    #[derive(Copy, Clone, Debug, moonwave_shader::std140::AsStd140)]
    #item

    impl #struct_ident {
      #(#shader_outputs_constants)*
    }

    impl moonwave_shader::UniformStruct for #struct_ident {
      fn get_id() -> moonwave_shader::Uuid {
        moonwave_shader::Uuid::from_u128_le(#uuid)
      }

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

      fn generate_dependencies() -> Vec<(String, Vec<(String, moonwave_shader::ShaderType)>)> {
        vec![#(#struct_dependencies),*]
      }
    }

    impl moonwave_core::BindGroupLayoutSingleton for #struct_ident {
      fn get_bind_group_lazy() -> moonwave_resources::ResourceRc<moonwave_resources::BindGroupLayout> {
        static cell: moonwave_core::OnceCell<moonwave_resources::ResourceRc<moonwave_resources::BindGroupLayout>> = moonwave_core::OnceCell::new();
        cell.get_or_init(|| {
          let desc = moonwave_resources::BindGroupLayoutDescriptor::new()
            .add_entry(0, moonwave_resources::BindGroupLayoutEntryType::UniformBuffer);
          let layout = moonwave_core::Core::get_instance().create_bind_group_layout(desc);
          layout
        }).clone()
      }
    }
  })
}
