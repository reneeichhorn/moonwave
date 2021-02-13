#version 450 

layout (location = 0) out vec2 v_uv;

void main()  {
  vec2 base = vec2((gl_VertexIndex << 1) & 2, gl_VertexIndex & 2);
  gl_Position = vec4(base * 2.0 + -1.0, 0.0, 1.0);
  v_uv = vec2(base.x, 1.0 - base.y);
}