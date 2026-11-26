// Box-filtered mip level from the level above, for all three arrays.
@group(0) @binding(0) var src_disp:texture_2d_array<f32>;
@group(0) @binding(1) var src_slope:texture_2d_array<f32>;
@group(0) @binding(2) var src_moment:texture_2d_array<f32>;
@group(0) @binding(3) var dst_disp:texture_storage_2d_array<rgba16float,write>;
@group(0) @binding(4) var dst_slope:texture_storage_2d_array<rgba16float,write>;
@group(0) @binding(5) var dst_moment:texture_storage_2d_array<rgba16float,write>;
fn box(t:texture_2d_array<f32>,p:vec2i,layer:i32)->vec4f {
    return 0.25*(textureLoad(t,p,layer,0)+textureLoad(t,p+vec2i(1,0),layer,0)
        +textureLoad(t,p+vec2i(0,1),layer,0)+textureLoad(t,p+vec2i(1,1),layer,0));
}
@compute @workgroup_size(8,8,1)
fn downsample(@builtin(global_invocation_id) id:vec3u) {
    let size=textureDimensions(dst_disp);
    if id.x>=size.x || id.y>=size.y {return;}
    let p=vec2i(id.xy*2u);
    let layer=i32(id.z);
    let q=vec2u(id.xy);
    textureStore(dst_disp,q,id.z,box(src_disp,p,layer));
    textureStore(dst_slope,q,id.z,box(src_slope,p,layer));
    // Moments of the slope average linearly, so the variance of the
    // filtered slope (E[s²] - E[s]²) grows with distance: roughness.
    textureStore(dst_moment,q,id.z,box(src_moment,p,layer));
}
