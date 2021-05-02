use dds::*;
use std::io::Cursor;

use moonwave_common::Vector2;
use moonwave_core::optick;
use moonwave_core::{rayon::prelude::*, Core};
use moonwave_resources::{SampledTexture, TextureFormat, TextureUsage};
use thiserror::Error;

pub enum TextureCodec {
  DDS,
}

pub fn create_static_texture(
  decoder: TextureCodec,
  data: &[u8],
) -> Result<SampledTexture, TextureReadError> {
  optick::event!("scene::texture::create_static_texture");

  let (width, height, buffer, format, row_size) = match decoder {
    TextureCodec::DDS => {
      // Parse dds.
      let mut cursor = Cursor::new(data);
      let dds = DDS::decode(&mut cursor).map_err(|_| TextureReadError::UnexpectedData)?;
      let width = dds.header.width;
      let height = dds.header.height;

      // Calculate needed byte size for
      let row_size = width * 4;
      let align = 256;
      let required_padding = (align - row_size % align) % align;
      let actual_row_size = row_size as usize + required_padding as usize;
      let mut buffer = vec![0u8; height as usize * actual_row_size];
      let row_chunks = buffer.chunks_exact_mut(actual_row_size);

      dds.layers[0]
        .par_chunks_exact(width as usize)
        .into_par_iter()
        .zip(row_chunks.collect::<Vec<_>>())
        .for_each(|(pixels, row_buffer)| {
          for (i, pixel) in pixels.iter().enumerate() {
            let base = i * 4;
            row_buffer[base] = pixel.r;
            row_buffer[base + 1] = pixel.g;
            row_buffer[base + 2] = pixel.b;
            row_buffer[base + 3] = pixel.a;
          }
        });
      (
        width,
        height,
        buffer,
        TextureFormat::Rgba8Unorm,
        actual_row_size,
      )
    }
  };

  let texture = Core::get_instance().create_inited_sampled_texture(
    None,
    TextureUsage::SAMPLED,
    format,
    Vector2::new(width, height),
    &buffer,
    row_size,
  );

  Ok(texture)
}

#[derive(Error, Debug)]
pub enum TextureReadError {
  #[error("Data is corrupted or unexpected texture codec used.")]
  UnexpectedData,
}
