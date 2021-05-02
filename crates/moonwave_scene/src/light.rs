use std::f32::consts::PI;

use legion::{world::SubWorld, *};
use moonwave_common::*;
use moonwave_core::{Core, SystemFactory, SystemStage, WrappedSystem};
use moonwave_shader::{uniform, ShaderNode, ShaderType};

use crate::{GenericUniform, Uniform};

pub const MAX_DIRECTIONAL_LIGHTS: usize = 1;

pub enum LightIntensity {
  Lumen(f32),
  Bulb { watts: f32, efficiency: f32 },
}

impl LightIntensity {
  pub fn as_lumen(&self) -> f32 {
    match self {
      LightIntensity::Lumen(lumen) => *lumen,
      LightIntensity::Bulb { watts, efficiency } => efficiency * 683.0 * watts,
    }
  }
}

pub struct DirectionalLight {
  pub intensity: LightIntensity,
  pub color: Vector3<f32>,
  pub direction: Vector3<f32>,
}

impl DirectionalLight {
  pub fn new() -> Self {
    Self {
      intensity: LightIntensity::Lumen(100.0),
      color: Vector3::new(1.0, 1.0, 1.0),
      direction: Vector3::new(0.0, -1.0, 0.0),
    }
  }

  pub fn set_from_real_world(
    &mut self,
    longitude: f32,
    latitude: f32,
    year: usize,
    month: usize,
    day: usize,
    hour: usize,
    minutes: usize,
  ) {
    // Calculate julian time.
    let mut julian_date = {
      let a = (14 - month) / 12;
      let y = year + 4800 - a;
      let m = month + 12 * a - 3;
      day + (153 * m + 2) / 5 + y * 365 + y / 4 - y / 100 + y / 400 - 32045
    };
    let mut julian_centuries = julian_date as f32 / 36525.0;
    let sidereal_time_hours = 6.6974 + 2400.0513 * julian_centuries;
    let total_hours =
      ((356 * year + 30 * month + day) * 24 + hour) as f32 + (minutes as f32) / 60.0;
    let sidereal_time_ut = sidereal_time_hours + (366.2422 / 365.2422) * total_hours;
    let sidereal_time = sidereal_time_ut * 15.0 + longitude;
    julian_date += (total_hours / 24.0) as usize;
    julian_centuries = julian_date as f32 / 36525.0;

    // Solar coordinates
    let mean_longitude = correct_angle(Deg(280.466 + 36000.77 * julian_centuries).into()).0;
    let mean_anomaly = correct_angle(Deg(357.529 + 35999.05 * julian_centuries).into()).0;
    let eq_center = Rad::from(Deg(
      (1.915 - 0.005 * julian_centuries) * mean_anomaly.sin() + 0.02 * (2.0 * mean_anomaly).sin(),
    ))
    .0;
    let eliptical_longitude = correct_angle(Rad(mean_longitude + eq_center)).0;
    let obliquity = Rad::from(Deg(23.439 - 0.013 * julian_centuries)).0;

    // Right Ascension
    let right_ascension =
      (obliquity.cos() * eliptical_longitude.sin()).atan2(eliptical_longitude.cos());
    let declination = (right_ascension.sin() * obliquity.sin()).asin();

    // Horizontal Coordinates
    let latitude_rad = Rad::from(Deg(latitude)).0;
    let mut hour_angle = correct_angle(Rad::from(Deg(sidereal_time)) - Rad(right_ascension)).0;
    if hour_angle > PI {
      hour_angle -= 2.0 * PI;
    }
    let altitude = (latitude_rad.sin() * declination.sin()
      + latitude_rad.cos() * declination.cos() * hour_angle.cos())
    .asin();

    // Nominator and denominator for calculating Azimuth
    // angle. Needed to test which quadrant the angle is in.
    let azi_nom = -hour_angle.sin();
    let azi_denom = declination.tan() * latitude_rad.cos() - latitude_rad.sin() * hour_angle.cos();
    let mut azi_muth = (azi_nom / azi_denom).atan();
    if azi_denom < 0.0 {
      azi_muth += PI;
    } else if azi_nom < 0.0 {
      azi_muth += 2.0 * PI;
    }

    // Build eulers
    let euler = Euler::new(Rad(altitude), Rad(azi_muth), Rad(0.0));
    let forward = Vector3::new(0.0, 1.0, 0.0);
    let direction = Quaternion::from(euler) * forward;
    self.direction = direction.xzy();
    self.intensity = LightIntensity::Lumen(inv_lerp(-12.0, 0.0, Deg::from(euler.x).0) * 1000.0);
  }
}

#[uniform]
struct DirectionalLightUniform {
  color_intensity: Vector4<f32>,
  direction: Vector3<f32>,
  enabled: u32,
}

#[uniform]
pub(crate) struct LightsUniform {
  directional_lights: [DirectionalLightUniform; MAX_DIRECTIONAL_LIGHTS],
}

#[derive(Debug)]
pub(crate) struct DirectionalLightShaderNode;
impl DirectionalLightShaderNode {
  pub(crate) const INPUT_LIGHTS: usize = 0;
  pub(crate) const INPUT_PIXEL: usize = 1;
  pub(crate) const INPUT_SHADING_NORMAL: usize = 2;
  pub(crate) const INPUT_SHADING_VIEW: usize = 3;
  pub(crate) const INPUT_SHADING_NOV: usize = 4;
  pub(crate) const OUTPUT_COLOR: usize = 0;
}
impl ShaderNode for DirectionalLightShaderNode {
  fn get_outputs(&self) -> Vec<ShaderType> {
    vec![ShaderType::Float3]
  }
  fn generate(&self, inputs: &[Option<String>], outputs: &[Option<String>], output: &mut String) {
    let in_lights = inputs[Self::INPUT_LIGHTS].as_ref().unwrap();
    let in_pixel = inputs[Self::INPUT_PIXEL].as_ref().unwrap();
    let in_normal = inputs[Self::INPUT_SHADING_NORMAL].as_ref().unwrap();
    let in_view = inputs[Self::INPUT_SHADING_VIEW].as_ref().unwrap();
    let in_nov = inputs[Self::INPUT_SHADING_NOV].as_ref().unwrap();
    let out_color = outputs[Self::OUTPUT_COLOR].as_ref().unwrap();

    *output += format!(
      r#"
      vec3 dir_color = vec3(0.0);
      for (int i = 0; i < {}; i++) {{
        DirectionalLightUniform light = {}[i];
        vec4 color_intensity = light.color_intensity;
        vec3 L = light.direction;
        float attentuation = 1.0;
        float NoL = saturate(dot({}, L));
        {}
        dir_color += color;
      }}
      vec3 {} = dir_color;
      "#,
      MAX_DIRECTIONAL_LIGHTS,
      in_lights,
      in_normal,
      surface_shading_normal(
        "color_intensity",
        "L",
        "attentuation",
        "NoL",
        in_view,
        in_normal,
        in_nov,
        in_pixel
      ),
      out_color,
    )
    .as_str();
  }
}

fn surface_shading_normal(
  color_intensity: &str,
  l: &str,
  attentuation: &str,
  nol: &str,
  view: &str,
  normal: &str,
  nov: &str,
  pixel: &str,
) -> String {
  format!(
    r#"
      Pixel pixel = {};
      vec3 h = normalize({} + {});
      float aNoL = saturate({});
      float NoV = {};
      float NoH = saturate(dot({}, h));
      float LoH = saturate(dot({}, h));
      float occlusion = 1.0;

      vec3 Fr = specularLobe(pixel.roughness, pixel.f0, h, NoV, aNoL, NoH, LoH);
      vec3 Fd = diffuseLobe(pixel.diffuse, pixel.roughness, NoV, aNoL, LoH);

      vec3 color = Fd + Fr * pixel.energyCompensation;
      color = (color * {}.rgb) *
            ({}.w * {} * aNoL * occlusion);
    "#,
    pixel,
    // h
    view,
    l,
    // nol
    nol,
    // nov
    nov,
    // noh
    normal,
    // loh
    l,
    // color
    color_intensity,
    color_intensity,
    attentuation,
  )
}

pub struct LightManager {
  uniform: Uniform<LightsUniform>,
}

static LIGHT_MANAGER_SYSTEM: std::sync::Once = std::sync::Once::new();

impl LightManager {
  pub fn new() -> Self {
    LIGHT_MANAGER_SYSTEM.call_once(|| {
      Core::get_instance()
        .get_world()
        .add_system_to_stage(LightManagerSystem, SystemStage::RenderingPreperations);
    });

    Self {
      uniform: Uniform::new(LightsUniform {
        directional_lights: [DirectionalLightUniform {
          enabled: 0,
          color_intensity: Vector4::zero(),
          direction: Vector3::zero(),
        }; 1],
      }),
    }
  }

  pub fn get_uniform(&self) -> GenericUniform {
    self.uniform.as_generic()
  }
}

#[system]
#[write_component(LightManager)]
#[read_component(DirectionalLight)]
pub fn light_manager_system(world: &mut SubWorld) {
  // Get active light manager.
  let light_manager = {
    let mut query = <&LightManager>::query();
    query.iter(world).next().unwrap()
  };

  // Update uniform with all active directional lights.
  let mut directional_lights_query = <&DirectionalLight>::query();
  for (index, light) in directional_lights_query.iter(world).enumerate() {
    if index >= MAX_DIRECTIONAL_LIGHTS {
      break;
    }
    let mut lights = light_manager.uniform.get_mut();
    lights.directional_lights[index].enabled = 1;
    lights.directional_lights[index].color_intensity = Vector4::new(
      light.color.x,
      light.color.y,
      light.color.z,
      light.intensity.as_lumen() * 0.005,
    );
    lights.directional_lights[index].direction = light.direction;
  }
}

struct LightManagerSystem;
impl SystemFactory for LightManagerSystem {
  fn create_system(&self) -> WrappedSystem {
    WrappedSystem(Box::new(light_manager_system_system()))
  }
}
