use std::{
  collections::{HashMap, VecDeque},
  hash::Hash,
  sync::Arc,
};

use moonwave_common::*;
use moonwave_core::Core;
use moonwave_resources::*;
use parking_lot::Mutex;

use crate::{create_raw_texture_data, TextureCodec};

pub struct DynamicTextureArray {
  textures: Vec<ResourceRc<Texture>>,
  texture_views: Vec<ResourceRc<TextureView>>,
  sampler: ResourceRc<Sampler>,
  bind_group: ResourceRc<BindGroup>,
  free_list: Mutex<VecDeque<usize>>,
}

impl DynamicTextureArray {
  pub fn new(dimension: Vector2<u32>, format: TextureFormat, mips: u32, size: usize) -> Self {
    let core = Core::get_instance();

    // Preallocate all textures
    let mut textures = Vec::with_capacity(size);
    let mut texture_views = Vec::with_capacity(size);
    for _ in 0..size {
      let texture = core.create_texture(
        None,
        TextureUsage::COPY_DST | TextureUsage::SAMPLED | TextureUsage::RENDER_ATTACHMENT,
        format,
        dimension,
        mips,
      );
      let view = core.create_texture_view(texture.clone());
      textures.push(texture);
      texture_views.push(view);
    }

    // Build sampler
    let sampler = core.create_sampler();

    // Build general purpose array texture sampler.
    let layout = core
      .get_gp_resources()
      .get_sampled_texture_array_bind_group_layout(size);

    // Build binding
    let bind_group = BindGroupDescriptor::new(layout)
      .add_texture_array_binding(0, texture_views.clone())
      .add_sampler_binding(1, sampler.clone());

    let bind_group = core.create_bind_group(bind_group);

    // Build free list
    let free_list = (0..size).collect::<VecDeque<_>>();

    Self {
      free_list: Mutex::new(free_list),
      textures,
      texture_views,
      bind_group,
      sampler,
    }
  }

  pub fn reserve(&self) -> Option<usize> {
    self.free_list.lock().pop_front()
  }

  pub fn load_into_spot(&self, decoder: TextureCodec, data: &[u8], index: usize) -> Option<usize> {
    // Put into texture array
    let (width, height, buffer, format, row_size) = create_raw_texture_data(decoder, data).unwrap();
    Core::get_instance().upload_texture(
      self.textures[index].clone(),
      TextureUsage::COPY_DST | TextureUsage::SAMPLED | TextureUsage::RENDER_ATTACHMENT,
      format,
      Vector2::new(width, height),
      &buffer,
      row_size,
    );

    Some(index)
  }

  pub fn load_into(&self, decoder: TextureCodec, data: &[u8]) -> Option<usize> {
    let index = self.reserve()?;
    self.load_into_spot(decoder, data, index)
  }

  pub fn remove(&self, index: usize) {
    self.free_list.lock().push_back(index);
  }
}

pub struct DynamicTextureHashMap<K: Hash> {
  texture_array: Arc<DynamicTextureArray>,
  index_mapping: Mutex<HashMap<K, usize>>,
  references: Mutex<HashMap<K, usize>>,
}

impl<K: Hash + Eq + Clone + Send + Sync + 'static> DynamicTextureHashMap<K> {
  pub fn new(dimension: Vector2<u32>, format: TextureFormat, mips: u32, size: usize) -> Self {
    Self {
      texture_array: Arc::new(DynamicTextureArray::new(dimension, format, mips, size)),
      index_mapping: Mutex::new(HashMap::new()),
      references: Mutex::new(HashMap::new()),
    }
  }

  pub fn get_binding(&self) -> ResourceRc<BindGroup> {
    self.texture_array.bind_group.clone()
  }

  pub fn remove_usage(&self, key: &K) {
    let mut references = self.references.lock();
    if let Some(reference) = references.get_mut(key) {
      *reference -= 1;

      if *reference == 0 {
        references.remove(key);

        let mut index_mapping = self.index_mapping.lock();
        let index = index_mapping.remove(key).unwrap();

        self.texture_array.remove(index)
      }
    }
  }

  pub fn load_into<F: FnOnce() -> (TextureCodec, Vec<u8>) + Send + Sync + 'static>(
    &self,
    key: K,
    getter: F,
  ) -> Option<usize> {
    let index = {
      let mut index_mapping = self.index_mapping.lock();
      let mut references = self.references.lock();

      // A texture with the key has been loaded already.
      if let Some(index) = index_mapping.get(&key) {
        *references.get_mut(&key).unwrap() += 1;
        return Some(*index);
      }

      let index = self.texture_array.reserve()?;
      index_mapping.insert(key.clone(), index);
      references.insert(key, 1);
      index
    };

    // Load texture
    let texture_array = self.texture_array.clone();
    Core::get_instance().spawn_background_task(move || {
      let (decoder, data) = getter();
      texture_array.load_into_spot(decoder, &data, index);
    });

    Some(index)
  }
}
