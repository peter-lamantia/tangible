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

@group(1) @binding(0) var base_texture: texture_2d<f32>;
@group(1) @binding(1) var base_sampler: sampler;

@group(1) @binding(2) var mr_texture: texture_2d<f32>;
@group(1) @binding(3) var mr_sampler: sampler;
@group(1) @binding(4) var normal_texture: texture_2d<f32>;
@group(1) @binding(5) var normal_sampler: sampler;
@group(1) @binding(6) var occlusion_texture: texture_2d<f32>;
@group(1) @binding(7) var occlusion_sampler: sampler;
@group(1) @binding(8) var emissive_texture: texture_2d<f32>;
@group(1) @binding(9) var emissive_sampler: sampler;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) surface: vec4<f32>,
    @location(5) emissive: vec3<f32>,
    @location(6) mr_uv: vec2<f32>,
    @location(7) normal_uv: vec2<f32>,
    @location(8) occlusion_uv: vec2<f32>,
    @location(9) emissive_uv: vec2<f32>,
    @location(10) map_factors: vec2<f32>,
    @location(11) tangent: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) world_pos: vec3<f32>,
    @location(2) barycentric: vec3<f32>,
    @location(3) color: vec4<f32>,
    @location(4) uv: vec2<f32>,
    @location(5) surface: vec4<f32>,
    @location(6) emissive: vec3<f32>,
    @location(7) mr_uv: vec2<f32>,
    @location(8) normal_uv: vec2<f32>,
    @location(9) occlusion_uv: vec2<f32>,
    @location(10) emissive_uv: vec2<f32>,
    @location(11) map_factors: vec2<f32>,
    @location(12) tangent: vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput, @builtin(vertex_index) index: u32) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = u.mvp * vec4<f32>(in.position, 1.0);
    out.world_normal = (u.normal_matrix * vec4<f32>(in.normal, 0.0)).xyz;
    out.barycentric = select(select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 1.0, 0.0), index % 3u == 1u), vec3<f32>(1.0, 0.0, 0.0), index % 3u == 0u);
    out.world_pos = (u.model * vec4<f32>(in.position, 1.0)).xyz;
    out.color = in.color; out.uv = in.uv; out.surface = in.surface; out.emissive = in.emissive;
    out.mr_uv = in.mr_uv; out.normal_uv = in.normal_uv; out.occlusion_uv = in.occlusion_uv; out.emissive_uv = in.emissive_uv; out.map_factors = in.map_factors;
    out.tangent = in.tangent;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var base = in.color * textureSample(base_texture, base_sampler, in.uv);
    let mr = textureSample(mr_texture,mr_sampler,in.mr_uv);
    let metallic = in.surface.x * mr.b;
    let roughness = clamp(in.surface.y * mr.g, 0.04, 1.0);
    let emissive = in.emissive * textureSample(emissive_texture,emissive_sampler,in.emissive_uv).rgb;
    let occlusion = mix(1.0,textureSample(occlusion_texture,occlusion_sampler,in.occlusion_uv).r,in.map_factors.y);
    var mapped_normal = textureSample(normal_texture,normal_sampler,in.normal_uv).xyz*2.0-1.0;
    mapped_normal = vec3<f32>(mapped_normal.xy*in.map_factors.x,mapped_normal.z);
    let dp1 = dpdx(in.world_pos);
    let dp2 = dpdy(in.world_pos);
    let duv1 = dpdx(in.normal_uv);
    let duv2 = dpdy(in.normal_uv);
    let widths = max(fwidth(in.barycentric),vec3<f32>(0.000001));
    if in.surface.z < -1.5 { base.a = 1.0; }
    if base.a < in.surface.z { discard; }
    if in.surface.z >= 0.0 { base.a = 1.0; }
    var n = normalize(in.world_normal + vec3<f32>(0.0,0.0,0.000001));
    let dp2perp = cross(dp2,n);
    let dp1perp = cross(n,dp1);
    var t = dp2perp*duv1.x + dp1perp*duv2.x;
    var b = dp2perp*duv1.y + dp1perp*duv2.y;
    if dot(in.tangent.xyz,in.tangent.xyz) > 0.000001 {
        // Prefer authored tangents, including mirrored-UV handedness. Remove
        // any normal component introduced by nonuniform scale/interpolation.
        let projected = in.tangent.xyz-n*dot(n,in.tangent.xyz);
        let length_squared = dot(projected,projected);
        if length_squared > 0.000001 {
            t = projected*inverseSqrt(length_squared);
            b = cross(n,t)*in.tangent.w;
        }
    }
    let scale = inverseSqrt(max(max(dot(t,t),dot(b,b)),0.000000000001));
    n = normalize(mat3x3<f32>(t*scale,b*scale,n)*mapped_normal);
    let light_dir = normalize(u.light_dir);
    // Preserve the viewer's two-sided lighting for back-facing surfaces.
    if dot(n,light_dir) < 0.0 { n = -n; }
    let diffuse = max(dot(n,light_dir),0.0)*u.light_color;
    let view_dir = normalize(u.eye_pos-in.world_pos);
    let half_dir = normalize(light_dir+view_dir);
    let spec = pow(max(dot(n,half_dir),0.0),mix(128.0,2.0,roughness));
    let specular = spec*u.light_color*mix(vec3<f32>(0.04),base.rgb,metallic);
    let result = select((u.ambient*occlusion+diffuse*(1.0-metallic))*base.rgb+specular+emissive,base.rgb,in.surface.w>0.5);
    let edge = min(min(smoothstep(0.0,widths.x*1.2,in.barycentric.x),smoothstep(0.0,widths.y*1.2,in.barycentric.y)),smoothstep(0.0,widths.z*1.2,in.barycentric.z));
    return vec4<f32>(mix(result,mix(vec3<f32>(0.08),result,edge),u.edges*(1.0-in.surface.w)),base.a);
}
