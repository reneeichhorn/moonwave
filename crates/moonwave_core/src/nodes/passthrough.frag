#version 450

layout(set = 0, binding = 0) uniform texture2D t_source;
layout(set = 0, binding = 1) uniform sampler s_source;

layout(location=0) in vec2 v_uv;

layout(location=0) out vec4 f_color;

void main() {
  vec4 source = texture(sampler2D(t_source, s_source), v_uv);
	f_color = source;
}