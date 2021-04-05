use std::io::Cursor;

use image::io::Reader;
use image::GenericImageView;
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

  #[allow(clippy::single_match)]
  let format = match decoder {
    TextureCodec::DDS => image::ImageFormat::Dds,
  };

  let image = Reader::with_format(Cursor::new(data), format)
    .decode()
    .map_err(|_| TextureReadError::UnexpectedData)?;

  let (width, height) = image.dimensions();

  // Create row pixel buffer with correct alignment and padding.
  let (buffer, format, row_size) = match image {
    image::DynamicImage::ImageRgb8(img) => {
      // Calculate needed byte size for
      let row_size = width * 4;
      let align = 256;
      let required_padding = (align - row_size % align) % align;
      let actual_row_size = row_size as usize + required_padding as usize;
      let mut buffer = vec![0u8; height as usize * actual_row_size];
      let row_chunks = buffer.chunks_exact_mut(actual_row_size);

      let rows = img.rows().into_iter().collect::<Vec<_>>();
      rows
        .into_par_iter()
        .zip(row_chunks.collect::<Vec<_>>())
        .for_each(|(pixels, row_buffer)| {
          for (i, pixel) in pixels.enumerate() {
            let base = i * 4;
            row_buffer[base] = pixel.0[0];
            row_buffer[base + 1] = pixel.0[1];
            row_buffer[base + 2] = pixel.0[2];
            row_buffer[base + 3] = 255;
          }
        });
      (buffer, TextureFormat::Rgba8Unorm, actual_row_size)
    }
    _ => unimplemented!("The image format is not yet implemented"),
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
