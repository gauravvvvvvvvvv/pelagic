// GPU ocean detail (Tessendorf 2001): time evolution of the initial
// spectrum, 256-point inverse FFTs along rows then columns in workgroup
// memory, and assembly into displacement / slope / moment textures with
// persistent foam. Three cascades are layers of the same arrays.
const N:u32=256u;
const G:f32=9.80665;
struct Params {
    // time (s, already reduced modulo the repeat period), frame step (s),
    // choppiness, depth (m)
    state:vec4f,
    // cascade tile sizes (m); w = frequency quantum 2π/repeat
    sizes:vec4f,
    // Jacobian foam thresholds per cascade; w = foam decay time (s)
    foam:vec4f,
    // Jacobian ramp width per cascade; w = foam enabled (0/1)
    ramp:vec4f,
};
@group(0) @binding(0) var<uniform> params:Params;
@group(0) @binding(1) var<storage,read> spectrum:array<vec4f>;
@group(0) @binding(2) var<storage,read_write> fields:array<vec2f>;
@group(0) @binding(3) var<storage,read_write> foam:array<f32>;
@group(0) @binding(4) var disp_out:texture_storage_2d_array<rgba16float,write>;
@group(0) @binding(5) var slope_out:texture_storage_2d_array<rgba16float,write>;
@group(0) @binding(6) var moment_out:texture_storage_2d_array<rgba16float,write>;

fn cmul(a:vec2f,b:vec2f)->vec2f {return vec2f(a.x*b.x-a.y*b.y,a.x*b.y+a.y*b.x);}
fn times_i(a:vec2f)->vec2f {return vec2f(-a.y,a.x);}
fn field(c:u32,f:u32,x:u32,y:u32)->u32 {return ((c*4u+f)*N+y)*N+x;}
fn signed(i:u32)->f32 {return select(f32(i),f32(i)-f32(N),i>=N/2u);}

// h(k,t) = h0(k) e^{iωt} + conj(h0(-k)) e^{-iωt}, then the spectra of the
// eight real fields packed pairwise as A + iB.
@compute @workgroup_size(16,16,1)
fn evolve(@builtin(global_invocation_id) id:vec3u) {
    let x=id.x;let y=id.y;let c=id.z;
    let s=spectrum[(c*N+y)*N+x];
    let k=vec2f(signed(x),signed(y))*(6.2831853/params.sizes[c]);
    let kl=length(k);
    var h=vec2f(0.);
    var unit=vec2f(0.);
    var inv=0.;
    if kl>1e-6 {
        // Dispersion quantised to the repeat period's harmonics.
        let w=sqrt(G*kl*tanh(min(kl*params.state.w,20.)));
        let quantum=params.sizes.w;
        let wq=floor(w/quantum)*quantum;
        let phase=wq*params.state.x;
        let e=vec2f(cos(phase),sin(phase));
        h=cmul(s.xy,e)+cmul(s.zw,vec2f(e.x,-e.y));
        unit=k/kl;
        inv=1./kl;
    }
    let ih=times_i(h);
    let dx=ih*unit.x;
    let dy=ih*unit.y;
    let sx=ih*k.x;
    let sy=ih*k.y;
    let jxx=-h*(k.x*k.x*inv);
    let jyy=-h*(k.y*k.y*inv);
    let jxy=-h*(k.x*k.y*inv);
    fields[field(c,0u,x,y)]=dx+times_i(dy);
    fields[field(c,1u,x,y)]=h+times_i(jxx);
    fields[field(c,2u,x,y)]=sx+times_i(sy);
    fields[field(c,3u,x,y)]=jyy+times_i(jxy);
}

var<workgroup> line:array<vec2f,256>;
fn reverse8(i:u32)->u32 {return reverseBits(i)>>24u;}
// In-place radix-2 inverse transform of `line` (bit-reversed input).
fn butterflies(t:u32) {
    for(var s=0u;s<8u;s=s+1u){
        let half=1u<<s;
        let pos=t%half;
        let i0=(t/half)*half*2u+pos;
        let i1=i0+half;
        let angle=6.2831853*f32(pos)/f32(half*2u);
        let v=cmul(vec2f(cos(angle),sin(angle)),line[i1]);
        let u=line[i0];
        line[i0]=u+v;
        line[i1]=u-v;
        workgroupBarrier();
    }
}
// One workgroup per (row, field, cascade).
@compute @workgroup_size(128)
fn fft_rows(@builtin(workgroup_id) wg:vec3u,@builtin(local_invocation_index) t:u32) {
    let base=field(wg.z,wg.y,0u,wg.x);
    line[reverse8(t)]=fields[base+t];
    line[reverse8(t+128u)]=fields[base+t+128u];
    workgroupBarrier();
    butterflies(t);
    fields[base+t]=line[t];
    fields[base+t+128u]=line[t+128u];
}
// One workgroup per (column, field, cascade).
@compute @workgroup_size(128)
fn fft_columns(@builtin(workgroup_id) wg:vec3u,@builtin(local_invocation_index) t:u32) {
    let base=field(wg.z,wg.y,wg.x,0u);
    line[reverse8(t)]=fields[base+t*N];
    line[reverse8(t+128u)]=fields[base+(t+128u)*N];
    workgroupBarrier();
    butterflies(t);
    fields[base+t*N]=line[t];
    fields[base+(t+128u)*N]=line[t+128u];
}

@compute @workgroup_size(16,16,1)
fn assemble(@builtin(global_invocation_id) id:vec3u) {
    let x=id.x;let y=id.y;let c=id.z;
    let f0=fields[field(c,0u,x,y)];
    let f1=fields[field(c,1u,x,y)];
    let f2=fields[field(c,2u,x,y)];
    let f3=fields[field(c,3u,x,y)];
    let chop=params.state.z;
    let jxx=f1.y;let jyy=f3.x;let jxy=f3.y;
    let jacobian=(1.+chop*jxx)*(1.+chop*jyy)-chop*chop*jxy*jxy;
    let inject=clamp((params.foam[c]-jacobian)/max(params.ramp[c],1e-4),0.,1.)*params.ramp.w;
    let i=(c*N+y)*N+x;
    let kept=foam[i]*exp(-params.state.y/max(params.foam.w,0.1));
    let value=clamp(max(kept,inject),0.,1.);
    foam[i]=value;
    let p=vec2u(x,y);
    textureStore(disp_out,p,c,vec4f(f0.x*chop,f0.y*chop,f1.x,value));
    textureStore(slope_out,p,c,vec4f(f2.x,f2.y,jxx,jyy));
    textureStore(moment_out,p,c,vec4f(f2.x*f2.x,f2.y*f2.y,jxy,0.));
}
