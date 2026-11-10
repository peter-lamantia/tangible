// WGSL Shader — Blinn-Phong lighting with two-sided normals

struct Uniforms {
    mvp: mat4x4<f32>,
    model: mat4x4<f32>,
    normal_matrix: mat4x4<f32>,
    light_dir: vec3<f32>,
    _pad1: f32,
    light_color: vec3<f32>,
    _pad2: f32,
    ambient: vec3<f32>,
    _pad3: f32,
    eye_pos: vec3<f32>,
    edges: f32,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) world_pos: vec3<f32>,
    @location(2) barycentric: vec3<f32>,
};

@vertex
fn vs_main(in: VertexInput, @builtin(vertex_index) index: u32) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = u.mvp * vec4<f32>(in.position, 1.0);
    out.world_normal = (u.normal_matrix * vec4<f32>(in.normal, 0.0)).xyz;
    out.barycentric = select(select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 1.0, 0.0), index % 3u == 1u), vec3<f32>(1.0, 0.0, 0.0), index % 3u == 0u);
    out.world_pos = (u.model * vec4<f32>(in.position, 1.0)).xyz;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let normal = normalize(in.world_normal);
    let light_dir = normalize(u.light_dir);

    // Two-sided lighting: flip normal if facing away from light
    var n = normal;
    if (dot(n, light_dir) < 0.0) {
        n = -n;
    }

    // Diffuse
    let diff = max(dot(n, light_dir), 0.0);
    let diffuse = diff * u.light_color;

    // Specular (Blinn-Phong)
    let view_dir = normalize(u.eye_pos - in.world_pos);
    let half_dir = normalize(light_dir + view_dir);
    let spec = pow(max(dot(n, half_dir), 0.0), 32.0);
    let specular = spec * u.light_color * 0.3;

    // Combine with a neutral object color
    let object_color = vec3<f32>(0.7, 0.75, 0.8);
    let result = (u.ambient + diffuse + specular) * object_color;

    let widths = fwidth(in.barycentric);
    let edge = min(min(smoothstep(0.0, widths.x * 1.2, in.barycentric.x), smoothstep(0.0, widths.y * 1.2, in.barycentric.y)), smoothstep(0.0, widths.z * 1.2, in.barycentric.z));
    return vec4<f32>(mix(result, mix(vec3<f32>(0.08), result, edge), u.edges), 1.0);
}
