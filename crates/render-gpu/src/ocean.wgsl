// Pelagic GPU renderer: physically based clear-sky atmosphere and clouds,
// FFT ocean surface on a camera-projected grid, PBR terrain, corals and
// vessels, underwater optics with volumetric light shafts, bloom and tone
// mapping. A visual preview, not a calibrated optical sensor.
const PI:f32=3.14159265;
const NEAR:f32=0.1;
const EARTH_R:f32=6371000.;
// View distance (m) from reversed depth.
fn linear_depth(d:f32)->f32 {return NEAR/max(d,1e-9);}
// Drop of the Earth's surface below the eye's tangent plane at a point.
fn curvature_drop(p:vec3f)->f32 {let r=length(p.xy-camera.eye.xy);return r*r/(2.*EARTH_R);}
struct Camera {
    eye:vec4f, forward:vec4f, right:vec4f, up:vec4f,
    sun:vec4f,
    // time (s), optical attenuation (1/m), underwater (0/1), exposure
    environment:vec4f,
    // width, height, tan(fov_y/2), aspect
    screen:vec4f,
    light0:vec4f, light1:vec4f, light2:vec4f, light3:vec4f, shadow:vec4f,
    // cloud cover 0-1, visibility (m), sea-state roughness hint, legacy sun
    sky:vec4f,
    // FFT cascade tile sizes (m), detail choppiness
    ocean:vec4f,
    // whitecap Jacobian threshold, ramp, unresolved slope variance, enabled
    foam:vec4f,
    // persistent-foam weight per cascade, physics-wave choppiness
    foam_weights:vec4f,
    // projected grid columns, rows, -, ocean surface enabled
    grid:vec4f,
    // sunlight at the surface (rgb), sky irradiance on a horizontal plane
    sun_light:vec4f, sky_light:vec4f,
};
@group(0) @binding(0) var<uniform> camera:Camera;
// Physics wave components (kx, ky, amplitude, phase at the current time):
// header = (count, significant height, whitecap fraction, wake count).
// Wakes: per vessel (x, y, vx, vy), (length, beam, -, -).
struct Waves { header:vec4f, c:array<vec4f,64>, wakes:array<vec4f,16> };
@group(0) @binding(1) var<uniform> waves:Waves;
@group(1) @binding(0) var scene_color:texture_2d<f32>;
@group(1) @binding(1) var scene_sampler:sampler;
@group(1) @binding(2) var scene_depth:texture_depth_2d;
@group(1) @binding(3) var bloom_texture:texture_2d<f32>;
@group(2) @binding(0) var shadow_map:texture_depth_2d;
@group(2) @binding(1) var shadow_sampler:sampler_comparison;
@group(3) @binding(0) var ocean_disp:texture_2d_array<f32>;
@group(3) @binding(1) var ocean_slope:texture_2d_array<f32>;
@group(3) @binding(2) var ocean_moment:texture_2d_array<f32>;
@group(3) @binding(3) var ocean_sampler:sampler;
@group(3) @binding(4) var sky_lut:texture_2d<f32>;
@group(3) @binding(5) var lut_sampler:sampler;

struct Input {
    @location(0) position:vec3f, @location(1) normal:vec3f,
    @location(2) uv:vec2f, @location(3) albedo:vec3f,
    @location(4) roughness:f32, @location(5) metallic:f32, @location(6) kind:f32,
};
struct Varying {
    @builtin(position) clip:vec4f,
    @location(0) world:vec3f, @location(1) normal:vec3f,
    @location(2) uv:vec2f, @location(3) albedo:vec3f,
    @location(4) roughness:f32, @location(5) metallic:f32, @location(6) kind:f32,
};
fn to_clip(world:vec3f)->vec4f {
    let delta=world-camera.eye.xyz;
    let p=vec3f(dot(delta,camera.right.xyz),dot(delta,camera.up.xyz),dot(delta,camera.forward.xyz));
    // Reversed depth with no far plane (depth = near / distance): float
    // depth keeps ~7 significant digits at every range, so coasts and
    // seabed hundreds of kilometres away do not flicker against each other.
    return vec4f(p.x/(camera.screen.z*camera.screen.w),p.y/camera.screen.z,NEAR,p.z);
}

// ------------------------------------------------------------ noise
// Integer hash (PCG): exact for any cell index, unlike sin-based hashes
// whose precision collapses for large arguments on some GPUs.
fn pcg(v:u32)->u32 {let s=v*747796405u+2891336453u;let w=((s>>((s>>28u)+4u))^s)*277803737u;return (w>>22u)^w;}
fn hash2(p:vec2f)->f32 {let i=vec2i(floor(p));return f32(pcg(bitcast<u32>(i.x)*1597334677u^pcg(bitcast<u32>(i.y))))*(1./4294967296.);}
fn noise2(p:vec2f)->f32 {
    let i=floor(p);let f=fract(p);let u=f*f*(3.-2.*f);
    return mix(mix(hash2(i),hash2(i+vec2f(1.,0.)),u.x),mix(hash2(i+vec2f(0.,1.)),hash2(i+vec2f(1.,1.)),u.x),u.y);
}
fn fbm(p:vec2f,octaves:i32)->f32 {
    var q=p;var a=0.5;var s=0.;var w=0.;
    for(var i=0;i<octaves;i=i+1){
        s=s+a*noise2(q);w=w+a;
        q=vec2f(q.x*1.6-q.y*1.2,q.x*1.2+q.y*1.6)+vec2f(3.1,1.7);
        a=a*0.5;
    }
    return s/w;
}
// Distance to the nearest of a jittered grid of points (cellular noise).
fn cells(p:vec2f)->f32 {
    let i=floor(p);let f=fract(p);
    var d=8.;
    for(var y=-1;y<=1;y=y+1){
        for(var x=-1;x<=1;x=x+1){
            let o=vec2f(f32(x),f32(y));
            let c=o+vec2f(hash2(i+o),hash2(i+o+vec2f(17.,31.)))-f;
            d=min(d,dot(c,c));
        }
    }
    return sqrt(d);
}

// ------------------------------------------------------------ atmosphere
// Same constants as sky.rs: Rayleigh + maritime Mie + ozone, single
// scattering plus an isotropic multiple-scattering term.
const EARTH:f32=6360000.;
const TOP:f32=6460000.;
const RAYLEIGH:vec3f=vec3f(5.802e-6,13.558e-6,33.1e-6);
const MIE:f32=7.0e-6;
const MIE_EXT:f32=7.8e-6;
const OZONE:vec3f=vec3f(0.650e-6,1.881e-6,0.085e-6);
const SUN_POWER:f32=3.6;
fn extinction_at(h:f32)->vec3f {
    let hh=max(h,0.);
    let ozone=max(1.-abs(hh-25000.)/15000.,0.);
    return RAYLEIGH*exp(-hh/8000.)+vec3f(MIE_EXT*exp(-hh/1200.))+OZONE*ozone;
}
fn to_top(r:f32,mu:f32)->f32 {
    let disc=r*r*(mu*mu-1.)+TOP*TOP;
    return -r*mu+sqrt(max(disc,0.));
}
fn hits_ground(r:f32,mu:f32)->bool {return mu<0. && r*r*(mu*mu-1.)+EARTH*EARTH>=0.;}
fn transmittance_to_sun(r:f32,mu:f32)->vec3f {
    if hits_ground(r,mu) {return vec3f(0.);}
    let span=to_top(r,mu);
    var depth=vec3f(0.);
    for(var i=0;i<8;i=i+1){
        let t=(f32(i)+0.5)/8.*span;
        let rr=sqrt(r*r+t*t+2.*r*t*mu);
        depth=depth+extinction_at(rr-EARTH)*span/8.;
    }
    return exp(-depth);
}
fn mie_phase(nu:f32)->f32 {
    let g=0.8;
    return 3./(8.*PI)*(1.-g*g)*(1.+nu*nu)/((2.+g*g)*pow(1.+g*g-2.*g*nu,1.5));
}
fn atmosphere(ray:vec3f,sun:vec3f)->vec3f {
    let r=EARTH+2.;
    let mu=ray.z;
    var span=to_top(r,mu);
    if hits_ground(r,mu) {
        span=-r*mu-sqrt(max(r*r*(mu*mu-1.)+EARTH*EARTH,0.));
    }
    let nu=dot(ray,sun);
    let pr=3./(16.*PI)*(1.+nu*nu);
    let pm=mie_phase(nu);
    var depth=vec3f(0.);
    var light=vec3f(0.);
    for(var i=0;i<24;i=i+1){
        let a=f32(i)/24.;let b=f32(i+1)/24.;
        let t0=a*a*span;let t1=b*b*span;
        let dt=t1-t0;let t=0.5*(t0+t1);
        let p=vec3f(0.,0.,r)+ray*t;
        let rr=length(p);
        let h=rr-EARTH;
        let dr=exp(-max(h,0.)/8000.);let dm=exp(-max(h,0.)/1200.);
        let e=extinction_at(h);
        let view=exp(-(depth+e*dt*0.5));
        let to_sun=transmittance_to_sun(rr,dot(p/rr,sun));
        let scatter=RAYLEIGH*dr*pr+vec3f(MIE*dm*pm);
        let multiple=(RAYLEIGH*dr+vec3f(MIE*dm))*0.08*sqrt(max(sun.z,0.));
        light=light+view*(scatter*to_sun+multiple)*dt;
        depth=depth+e*dt;
    }
    return light*SUN_POWER;
}
// Sky-view table: u = azimuth from the sun, v = elevation with extra
// resolution at the horizon.
fn sun_azimuth()->f32 {return atan2(camera.sun.y,camera.sun.x);}
@fragment fn sky_table(v:Quad)->@location(0) vec4f {
    let azimuth=sun_azimuth()+v.uv.x*2.*PI;
    let s=(1.-v.uv.y)*2.-1.;
    let elevation=sign(s)*s*s*PI*0.5;
    let ray=vec3f(cos(elevation)*cos(azimuth),cos(elevation)*sin(azimuth),sin(elevation));
    return vec4f(atmosphere(ray,normalize(camera.sun.xyz)),1.);
}
fn sky_lookup(ray:vec3f)->vec3f {
    let d=normalize(ray);
    let u=fract((atan2(d.y,d.x)-sun_azimuth())/(2.*PI));
    let e=asin(clamp(d.z,-1.,1.));
    let s=sign(e)*sqrt(abs(e)/(PI*0.5));
    let v=1.-(s*0.5+0.5);
    return textureSampleLevel(sky_lut,lut_sampler,vec2f(u,v),0.).rgb;
}
// Clouds: one stratocumulus-like layer at 1.5 km advected by the wind.
fn cloud_density(p:vec2f,octaves:i32)->f32 {
    let cover=camera.sky.x;
    if cover<0.01 {return 0.;}
    let t=camera.environment.x;
    let f=fbm(p/2600.+vec2f(t*0.0021,t*0.0008),octaves);
    return smoothstep(1.02-cover*0.9,1.25-cover*0.9,f+0.18);
}
fn cloud_shadow(p:vec3f)->f32 {
    let s=normalize(camera.sun.xyz);
    if camera.sky.x<0.01 || s.z<0.02 {return 1.;}
    let q=p.xy+s.xy/s.z*(1500.-p.z);
    return 1.-0.75*cloud_density(q,3);
}
fn clouds(ray:vec3f,background:vec3f)->vec3f {
    if camera.sky.x<0.01 || ray.z<0.004 {return background;}
    let t=(1500.-camera.eye.z)/ray.z;
    if t<0. {return background;}
    let p=camera.eye.xy+ray.xy*t;
    let d=cloud_density(p,5);
    if d<=0.001 {return background;}
    let s=normalize(camera.sun.xyz);
    let toward=normalize(s.xy+vec2f(1e-4,0.));
    let shade=cloud_density(p+toward*260.,4);
    let lit=exp(-shade*2.2);
    let forward=1.+1.5*pow(max(dot(ray,s),0.),8.);
    let sun=camera.sun_light.rgb*(0.2+0.8*lit)*forward*0.32;
    let ambient=camera.sky_light.rgb*(0.55+0.45*(1.-d))*0.42;
    var color=(sun+ambient)*mix(1.,0.72,d);
    let alpha=clamp(d*1.6,0.,1.)*smoothstep(0.004,0.06,ray.z);
    // Aerial perspective towards the distant cloud deck.
    color=mix(color,background,1.-exp(-t/45000.));
    return mix(background,color,alpha);
}
fn sun_disc(ray:vec3f)->vec3f {
    let s=normalize(camera.sun.xyz);
    let c=dot(ray,s);
    let edge=cos(0.0047);
    if c<edge-0.0002 {return vec3f(0.);}
    // Limb darkening; radiance capped for half-float targets.
    let r=clamp((1.-c)/(1.-edge),0.,1.);
    return camera.sun_light.rgb*80.*(1.-0.6*(1.-sqrt(1.-r)))*smoothstep(edge-0.0002,edge+0.00005,c);
}
// Sky radiance without the sun disc, including clouds.
fn sky_full(ray:vec3f)->vec3f {
    var d=ray;
    if d.z<0. {d=normalize(vec3f(d.xy,0.));}
    return clouds(d,sky_lookup(d));
}
fn sky_ambient(n:vec3f)->vec3f {
    // Hemisphere: full sky irradiance facing up, ground/sea bounce below.
    return camera.sky_light.rgb*(0.55+0.45*n.z)+camera.sun_light.rgb*max(camera.sun.z,0.)*0.04*(1.-n.z);
}
// Aerial perspective over the sea (above water only).
fn aerial(color:vec3f,p:vec3f)->vec3f {
    let delta=p-camera.eye.xyz;
    let distance=length(delta);
    let visibility=max(camera.sky.y,1000.);
    let height=max(0.5*(p.z+camera.eye.z),0.);
    let density=exp(-height/1200.);
    let trans=exp(-distance*density*3.0/visibility);
    var dir=delta/max(distance,0.001);
    dir=normalize(vec3f(dir.xy,max(dir.z,0.015)));
    return color*trans+sky_lookup(dir)*(1.-trans);
}

// ------------------------------------------------------------ water optics
// Diffuse attenuation (1/m): red is absorbed within metres, blue travels
// furthest; scaled by the scene's optical attenuation (clarity).
fn clarity()->f32 {return max(camera.environment.y/0.035,0.2);}
fn absorption()->vec3f {return vec3f(0.45,0.065,0.03)*clarity();}
fn scattering()->vec3f {return vec3f(0.03,0.035,0.04)*clarity();}
fn light_at(d:f32)->vec3f {return exp(-(absorption()+scattering()*0.25)*max(d,0.));}
fn sun_underwater()->vec3f {
    let s=normalize(camera.sun.xyz);
    let h=length(s.xy)/1.333;
    return normalize(vec3f(s.xy*(h/max(length(s.xy),0.001)),sqrt(max(1.-h*h,0.))));
}
// Downwelling irradiance just below the surface.
fn surface_irradiance()->vec3f {
    let s=normalize(camera.sun.xyz);
    return camera.sun_light.rgb*max(s.z,0.)*0.97+camera.sky_light.rgb*0.93;
}
// Radiance of light scattered back towards a viewer by deep water at
// depth d (the asymptotic path radiance).
fn deep_water(d:f32)->vec3f {
    let c=absorption()+scattering();
    return surface_irradiance()*light_at(d)*scattering()/c*0.085;
}
fn heyney(nu:f32,g:f32)->f32 {return (1.-g*g)/(4.*PI*pow(1.+g*g-2.*g*nu,1.5));}
// Caustic network: focused sunlight from the waves, on upward surfaces.
fn caustic(p:vec2f,t:f32)->f32 {
    let a=cells(p*0.55+vec2f(t*0.21,t*0.13));
    let b=cells(p*0.83-vec2f(t*0.17,-t*0.19)+vec2f(5.2,1.3));
    let ridge=pow(1.-smoothstep(0.,0.55,abs(a-b)),3.);
    return ridge*2.2;
}
// Light shafts: surface focusing projected along the refracted sun.
fn shafts(p:vec3f,t:f32)->f32 {
    let sw=sun_underwater();
    let depth=max(-p.z,0.);
    let q=p.xy+sw.xy/max(sw.z,0.2)*depth;
    let n=noise2(q*0.32+vec2f(t*0.25,-t*0.15))*noise2(q*0.71-vec2f(t*0.21,t*0.27));
    return 0.25+3.0*pow(n,1.6);
}
// Underwater path radiance and attenuation from the eye to p.
fn underwater_fog(color:vec3f,p:vec3f)->vec3f {
    let delta=p-camera.eye.xyz;
    let distance=length(delta);
    let dir=delta/max(distance,0.0001);
    let a=absorption();let b=scattering();let c=a+b+vec3f(0.01);
    let t=camera.environment.x;
    let sw=sun_underwater();
    let phase=heyney(dot(dir,sw),0.72)+0.05;
    let reach=min(distance,70.);
    let steps=i32(clamp(camera.grid.z,3.,10.));
    let seg_len=reach/f32(steps);
    var trans=vec3f(1.);
    var light=vec3f(0.);
    let e0=surface_irradiance();
    let sun0=camera.sun_light.rgb*max(camera.sun.z,0.);
    let clouded=cloud_shadow(vec3f(camera.eye.xy,0.));
    for(var i=0;i<steps;i=i+1){
        let s=camera.eye.xyz+dir*(f32(i)+0.5)*seg_len;
        let depth=max(-s.z,0.);
        let down=light_at(depth);
        let beam=sun0*down*shafts(s,t)*clouded;
        let diffuse=e0*down*0.1;
        let seg=(vec3f(1.)-exp(-c*seg_len))/c;
        light=light+trans*b*(beam*phase+diffuse)*seg;
        trans=trans*exp(-c*seg_len);
    }
    if distance>reach {
        let far=deep_water(max(-p.z,0.)*0.5+max(-camera.eye.z,0.)*0.5);
        let rest=exp(-c*(distance-reach));
        light=light+trans*far*(vec3f(1.)-rest);
        trans=trans*rest;
    }
    return color*trans+light;
}

// ------------------------------------------------------------ geometry
@vertex fn geometry(input:Input)->Varying {
    var v=input;
    if abs(v.kind-5.)<0.25 {
        // Legacy flat water: same analytic sum as SeaState::sample.
        let p=v.position.xy;
        let reach=max(abs(p.x),abs(p.y));
        let spacing=select(0.,reach/8.,reach>513.);
        var h=0.;var slope=vec2f(0.);
        let count=min(u32(waves.header.x),64u);
        for(var i=0u;i<count;i=i+1u){
            let c=waves.c[i];
            let a=c.z*(1.-smoothstep(1.57,3.14,length(c.xy)*spacing));
            let phase=dot(c.xy,p)+c.w;
            h=h+a*cos(phase);
            slope=slope+a*c.xy*sin(phase);
        }
        v.position.z=h;v.normal=normalize(vec3f(slope,1.));v.kind=1.;
    }
    if abs(v.kind-9.)<0.25 {
        // Seagrass and kelp blades sway in the wave orbital motion; uv.y
        // is the height fraction along the blade, uv.x a per-blade phase.
        let t=camera.environment.x;
        let bend=v.uv.y*v.uv.y;
        let d=select(normalize(waves.c[0].xy+vec2f(1e-4,0.)),vec2f(0.8,0.6),waves.header.x<0.5);
        let sway=sin(t*0.9+v.uv.x+dot(v.position.xy,vec2f(0.21,0.17)))*0.18
            +sin(t*2.3+v.uv.x*2.7)*0.05;
        v.position=v.position+vec3f(d*sway*bend*max(waves.header.y*0.6,0.4),-abs(sway)*bend*0.1);
    }
    var o:Varying;
    // Drawn on the curved Earth (as the sea is), so distant coasts and
    // seabed sit where they should relative to the horizon.
    o.clip=to_clip(v.position-vec3f(0.,0.,curvature_drop(v.position)));
    o.world=v.position;o.normal=v.normal;o.uv=v.uv;o.albedo=v.albedo;
    o.roughness=v.roughness;o.metallic=v.metallic;o.kind=v.kind;return o;
}
// Animals: one mesh per species (model space: snout +x at 0.5, tail -x,
// left +y, up +z, length 1) drawn once per animal. Swimming is a travelling
// body wave by locomotion mode: 0 body-and-tail (most fish), 1 whole body
// (eels), 2 flapping wings (rays), 3 vertical flukes (whales, dolphins),
// 4 flipper strokes (turtles).
struct Animal {
    // position (m), body length (m)
    @location(7) place:vec4f,
    // heading (unit), phase
    @location(8) heading:vec4f,
    // mode, amplitude (body lengths), angular frequency (rad/s), wavenumber
    @location(9) swim:vec4f,
    // colour multiplier, body bend (turning)
    @location(10) tint:vec4f,
};
fn animal_local(p0:vec3f,a:Animal)->vec3f {
    var p=p0;
    let t=camera.environment.x;
    let mode=a.swim.x;let amp=a.swim.y;let w=a.swim.z;let k=a.swim.w;
    let behind=clamp(0.5-p.x,0.,1.);
    let wave=sin(w*t+a.heading.w-k*behind);
    if mode<0.5 {
        let tail=clamp((0.3-p.x)/0.8,0.,1.);
        p.y=p.y+amp*(0.15+tail*tail*0.85)*wave*smoothstep(-0.1,0.3,behind);
    } else if mode<1.5 {
        p.y=p.y+amp*(0.35+0.65*behind)*wave;
    } else if mode<2.5 {
        let span=abs(p.y)*2.;
        p.z=p.z+amp*span*span*sin(w*t+a.heading.w-k*behind*0.6);
    } else if mode<3.5 {
        let tail=clamp((0.2-p.x)/0.7,0.,1.);
        p.z=p.z+amp*tail*tail*wave;
    } else {
        let span=clamp(abs(p.y)*3.-0.35,0.,1.);
        p.z=p.z+amp*span*sin(w*t+a.heading.w);
    }
    // Turning bends the body along its length.
    p.y=p.y+a.tint.w*p.x*p.x;
    return p;
}
fn animal_basis(a:Animal)->mat3x3f {
    let f=normalize(a.heading.xyz+vec3f(1e-5,0.,0.));
    var r=cross(f,vec3f(0.,0.,1.));
    if length(r)<1e-3 {r=vec3f(0.,1.,0.);}
    r=normalize(r);
    let u=cross(r,f);
    return mat3x3f(f,-r,u);
}
@vertex fn creature(v:Input,a:Animal)->Varying {
    let basis=animal_basis(a);
    let world=a.place.xyz+basis*(animal_local(v.position,a)*a.place.w);
    var o:Varying;
    o.clip=to_clip(world-vec3f(0.,0.,curvature_drop(world)));
    o.world=world;o.normal=normalize(basis*v.normal);o.uv=v.uv;
    o.albedo=v.albedo*a.tint.rgb;
    o.roughness=v.roughness;o.metallic=v.metallic;o.kind=v.kind;return o;
}
@vertex fn creature_shadow(v:Input,a:Animal)->@builtin(position) vec4f {
    let basis=animal_basis(a);
    let world=a.place.xyz+basis*(animal_local(v.position,a)*a.place.w);
    return vec4f(light_clip(world),1.);
}
fn light_clip(p:vec3f)->vec3f {
    let q=vec4f(p,1.);
    return vec3f(dot(camera.light0,q),dot(camera.light1,q),dot(camera.light2,q));
}
@vertex fn shadow_vertex(v:Input)->@builtin(position) vec4f {
    return vec4f(light_clip(v.position),1.);
}
fn sun_visibility(p:vec3f,n:vec3f)->f32 {
    if camera.shadow.x<0.5 {return 1.;}
    let c=light_clip(p+n*camera.shadow.y*1.5);
    let uv=vec2f(c.x*0.5+0.5,0.5-c.y*0.5);
    if c.z>=1. || any(uv<vec2f(0.)) || any(uv>vec2f(1.)) {return 1.;}
    let texel=1./camera.shadow.z;
    var lit=0.;
    for(var y=-1;y<=1;y=y+1){
        for(var x=-1;x<=1;x=x+1){
            lit=lit+textureSampleCompareLevel(shadow_map,shadow_sampler,uv+vec2f(f32(x),f32(y))*texel,c.z-camera.shadow.w);
        }
    }
    let edge=smoothstep(0.85,1.,max(abs(c.x),abs(c.y)));
    return mix(lit/9.,1.,edge);
}
struct Quad { @builtin(position) position:vec4f,@location(0) uv:vec2f };
@vertex fn fullscreen(@builtin(vertex_index) i:u32)->Quad {
    let p=array<vec2f,3>(vec2f(-1.,-1.),vec2f(3.,-1.),vec2f(-1.,3.));
    var o:Quad;o.position=vec4f(p[i],0.,1.);o.uv=vec2f(p[i].x*0.5+0.5,0.5-p[i].y*0.5);return o;
}
fn view_ray(uv:vec2f)->vec3f {
    return normalize(camera.forward.xyz+camera.right.xyz*((uv.x*2.-1.)*camera.screen.z*camera.screen.w)+camera.up.xyz*((1.-uv.y*2.)*camera.screen.z));
}
@fragment fn sky(v:Quad)->@location(0) vec4f {
    let ray=view_ray(v.uv);
    if camera.environment.z>0.5 {
        return vec4f(underwater_fog(vec3f(0.),camera.eye.xyz+ray*400.),1.);
    }
    // The true horizon dips below horizontal with height.
    let dip=sqrt(2.*max(camera.eye.z,0.5)/EARTH_R);
    if ray.z< -dip {
        // Below the horizon beyond the drawn sea: distant water in haze.
        let horizon=sky_lookup(normalize(vec3f(ray.xy,0.02)));
        return vec4f(mix(deep_water(0.)*0.6+horizon*0.25,horizon,0.8),1.);
    }
    return vec4f(sky_full(ray)+sun_disc(ray),1.);
}
fn fog(color:vec3f,p:vec3f)->vec3f {
    if camera.environment.z>0.5 {return underwater_fog(color,p);}
    return aerial(color,p);
}

// ------------------------------------------------------------ surfaces
struct Surface { albedo:vec3f, normal:vec3f, roughness:f32, metallic:f32 };
// Seabed and land: material from height, slope and noise. The vertex
// albedo is a habitat tint (white = none).
fn terrain_surface(v:Varying,n:vec3f,footprint:f32)->Surface {
    var s:Surface;
    let p=v.world;
    let detail=1./(1.+footprint*footprint*6.);
    let slope=1.-clamp(n.z,0.,1.);
    let broad=fbm(p.xy*0.013,3);
    let fine=noise2(p.xy*0.9)*0.6+noise2(p.xy*3.7)*0.4;
    let rockiness=smoothstep(0.22,0.42,slope+(broad-0.5)*0.25);
    let rock=vec3f(0.27,0.26,0.24)*(0.65+0.55*fine)*mix(vec3f(1.),vec3f(1.1,1.,0.9),broad);
    var color:vec3f;
    var normal=n;
    var rough=0.9;
    if p.z< -0.4 {
        // Sand ripples in the shallows, silt and mud with depth.
        let sand=vec3f(0.66,0.58,0.42)*(0.82+0.35*broad);
        let mud=vec3f(0.30,0.31,0.26)*(0.8+0.3*broad);
        let depthmix=smoothstep(18.,70.,-p.z+(broad-0.5)*20.);
        let ripple=sin(dot(p.xy,vec2f(1.9,0.7))+noise2(p.xy*0.4)*3.)*detail*(1.-depthmix);
        color=mix(sand,mud,depthmix)*(1.+ripple*0.06+(fine-0.5)*0.14*detail);
        normal=normalize(n+vec3f(ripple*0.06,(fine-0.5)*0.05*detail,0.));
        color=mix(color,rock,rockiness)*v.albedo;
    } else {
        let height=p.z;
        let beach=vec3f(0.78,0.71,0.55)*(0.9+0.2*broad);
        let grass=mix(vec3f(0.17,0.25,0.08),vec3f(0.08,0.15,0.05),smoothstep(0.35,0.7,broad));
        let dry=vec3f(0.42,0.38,0.24);
        let veg=mix(grass,dry,smoothstep(300.,1200.,height+broad*300.))*(0.75+0.45*fine);
        let snow=vec3f(0.92,0.94,0.97);
        let shore=1.-smoothstep(1.5,4.+broad*4.,height);
        color=mix(veg,beach,shore*(1.-rockiness));
        color=mix(color,rock,rockiness);
        color=mix(color,snow,smoothstep(2400.,2900.,height+broad*400.)*(1.-rockiness*0.7));
        // Satellite colour where the scene has imagery (weight in metallic).
        color=mix(color,v.albedo*(0.82+0.36*fine),clamp(v.metallic,0.,1.));
        normal=normalize(n+vec3f((fine-0.5)*0.12*detail,(noise2(p.xy*2.1)-0.5)*0.12*detail,0.));
        // Wet sand just above the waterline.
        let wet=1.-smoothstep(0.,0.9,height);
        color=color*mix(1.,0.62,wet);
        rough=mix(0.9,0.45,wet);
    }
    s.albedo=color;s.normal=normal;s.roughness=rough;s.metallic=0.;
    return s;
}
fn coral_surface(v:Varying,n:vec3f)->Surface {
    var s:Surface;
    let p=v.world;
    let polyps=cells(p.xy*22.+p.z*13.);
    let bumps=noise2(p.xy*9.+vec2f(p.z*7.));
    s.albedo=v.albedo*(0.72+0.4*bumps)*(0.85+0.3*smoothstep(0.1,0.5,polyps));
    s.normal=normalize(n+vec3f(bumps-0.5,polyps-0.4,0.)*0.25);
    s.roughness=0.75;s.metallic=0.;
    return s;
}
fn shade(v:Varying,s:Surface,front_normal:vec3f)->vec3f {
    let n=s.normal;
    let base=s.albedo;
    let underwater=camera.environment.z>0.5 || v.world.z<0.;
    let view=normalize(camera.eye.xyz-v.world);
    let l=select(normalize(camera.sun.xyz),sun_underwater(),underwater);
    let half_vector=normalize(l+view);
    let nl=max(dot(n,l),0.);let nv=max(dot(n,view),0.001);let nh=max(dot(n,half_vector),0.);let vh=max(dot(view,half_vector),0.);
    let rough=clamp(s.roughness,0.06,1.);let alpha=rough*rough;let a2=alpha*alpha;
    let d=a2/(PI*pow(nh*nh*(a2-1.)+1.,2.)+0.00001);
    let k=pow(rough+1.,2.)/8.;let visibility=nv/(nv*(1.-k)+k)*nl/(nl*(1.-k)+k);
    let f0=mix(vec3f(0.04),base,s.metallic);let fresnel=f0+(1.-f0)*pow(1.-vh,5.);
    let spec=d*visibility*fresnel/(4.*nv*max(nl,0.001));
    let diffuse=(1.-fresnel)*(1.-s.metallic)*base/PI;
    let shadow=sun_visibility(v.world,front_normal)*cloud_shadow(v.world);
    if underwater {
        let depth=max(-v.world.z,0.);
        let sun=camera.sun_light.rgb*max(camera.sun.z,0.)*0.97*light_at(depth);
        let t=camera.environment.x;
        let roughness_of_sea=clamp(waves.header.y*1.5,0.35,1.2);
        let focus=caustic(v.world.xy*0.9+l.xy*depth*0.3,t*1.3)*clamp(n.z,0.,1.)*exp(-depth*0.04)*roughness_of_sea;
        let ambient=surface_irradiance()*light_at(depth)*(0.25+0.2*clamp(n.z,0.,1.));
        return diffuse*(sun*nl*shadow*(0.6+focus)/max(l.z,0.3)+ambient)+spec*nl*shadow*sun;
    }
    let sun=camera.sun_light.rgb;
    return (diffuse+spec)*nl*shadow*sun+diffuse*sky_ambient(n)+spec*0.;
}
@fragment fn opaque(v:Varying,@builtin(front_facing) front:bool)->@location(0) vec4f {
    var n=normalize(v.normal);if !front {n=-n;}
    let footprint=max(length(dpdx(v.world)),length(dpdy(v.world)));
    let uvf=max(length(dpdx(v.uv)),length(dpdy(v.uv)));
    var s:Surface;
    s.albedo=v.albedo;s.normal=n;s.roughness=v.roughness;s.metallic=v.metallic;
    if v.kind<0.5 {
        // Legacy sand: ripples plus fine grain.
        let detail=1./(1.+uvf*uvf*12.);
        let ripple=sin(v.uv.x*2.5+sin(v.uv.y*0.23)+noise2(v.uv*0.35)*2.);
        let grain=noise2(v.uv*9.)*0.6+noise2(v.uv*31.)*0.4;
        s.albedo=v.albedo*(1.+(ripple*0.06+(grain-0.5)*0.16)*detail);
        s.normal=normalize(n+vec3f(ripple*0.05*detail,(grain-0.5)*0.04*detail,0.));
    } else if abs(v.kind-6.)<0.25 {
        s=terrain_surface(v,n,footprint);
    } else if abs(v.kind-7.)<0.25 {
        s=coral_surface(v,n);
    } else if abs(v.kind-8.)<0.25 {
        // Sea fan: a lattice of fine branches.
        let g=abs(fract(v.uv*vec2f(14.,22.))-0.5);
        if min(g.x,g.y)>0.17 && hash2(floor(v.uv*vec2f(14.,22.)))>0.15 {discard;}
        s.albedo=v.albedo;s.roughness=0.8;
    } else if abs(v.kind-9.)<0.25 {
        s.albedo=v.albedo*(0.7+0.5*v.uv.y);s.roughness=0.65;
    }
    let color=shade(v,s,n);
    return vec4f(fog(color,v.world),1.);
}

// ------------------------------------------------------------ ocean surface
struct OceanOut {
    @builtin(position) clip:vec4f,
    @location(0) world:vec3f,
    @location(1) plane:vec2f,
    @location(2) slope:vec2f,
    // physics-wave divergence (for the Jacobian), height above mean level
    @location(3) extra:vec2f,
};
// Camera-projected grid: every vertex is a ray from the eye intersected
// with the mean sea level (from above or below), then displaced by the
// physics components and the FFT cascades. Resolution follows the screen,
// so there is no fixed mesh edge and the sea reaches the curved horizon.
@vertex fn ocean_grid(@builtin(vertex_index) index:u32)->OceanOut {
    let cols=u32(camera.grid.x);let rows=u32(camera.grid.y);
    let gx=index%cols;let gy=index/cols;
    let s=vec2f(f32(gx)/f32(cols-1u),f32(gy)/f32(rows-1u));
    let ndc=vec2f(mix(-1.15,1.15,s.x),mix(-1.35,1.12,s.y));
    let tan_y=camera.screen.z;
    let ray=normalize(camera.forward.xyz+camera.right.xyz*ndc.x*tan_y*camera.screen.w+camera.up.xyz*ndc.y*tan_y);
    let eye=camera.eye.xyz;
    let above=eye.z>=0.;
    let h=select(min(eye.z,-0.4),max(eye.z,0.4),above);
    let horizontal=length(ray.xy);
    let flat_dir=select(vec2f(1.,0.),ray.xy/max(horizontal,1e-6),horizontal>1e-6);
    // Above water: the ray meets the curved sea (z = -r^2/2R), and rays that
    // miss it end on the true horizon (sqrt(2Rh)), so the sea reaches the
    // horizon at any altitude without folding over itself.
    let horizon=min(sqrt(2.*EARTH_R*h+h*h),3000000.);
    var dist=horizon;
    if above {
        let a=horizontal*horizontal/(2.*EARTH_R);
        let disc=ray.z*ray.z-4.*a*h;
        if ray.z<0. && disc>0. {
            let t=select(-h/ray.z,(-ray.z-sqrt(disc))/(2.*a),a>1e-12);
            dist=min(t*horizontal,horizon);
        }
    } else {
        dist=40000.;
        if ray.z>1e-5 {dist=min(-h/ray.z*horizontal,40000.);}
    }
    let plane=eye.xy+flat_dir*dist;
    let slant=sqrt(dist*dist+h*h);
    let step_angle=2.5*tan_y/f32(rows-1u);
    // Screen footprint of one grid step across the view. Long waves fade
    // only when they would alias across the screen; the along-view footprint
    // is huge at grazing angles but crests there still show as silhouettes.
    let spacing=min(slant*step_angle,4000.);
    let filter_spacing=min(slant*step_angle/sqrt(max(abs(ray.z),0.02)),4000.);
    var height=0.;var disp=vec2f(0.);var grad=vec2f(0.);var div=0.;
    let chop=camera.foam_weights.w;
    let count=min(u32(waves.header.x),64u);
    for(var i=0u;i<count;i=i+1u){
        let c=waves.c[i];
        let kl=max(length(c.xy),1e-6);
        let a=c.z*(1.-smoothstep(1.57,3.14,kl*spacing));
        let phase=dot(c.xy,plane)+c.w;
        let sn=sin(phase);let cs=cos(phase);
        height=height+a*cs;
        grad=grad-a*c.xy*sn;
        disp=disp-c.xy/kl*a*sn*chop;
        div=div-a*kl*cs*chop;
    }
    var detail=vec3f(0.);
    if camera.grid.w>0.5 {
        for(var c=0;c<3;c=c+1){
            let size=camera.ocean[c];
            let texel=size/256.;
            let lod=max(log2(max(filter_spacing/texel,1.)),0.);
            let fade=1.-smoothstep(size*0.25,size*0.6,filter_spacing);
            detail=detail+textureSampleLevel(ocean_disp,ocean_sampler,plane/size,c,lod).xyz*fade;
        }
    }
    var world=vec3f(plane+disp+detail.xy,height+detail.z);
    world.z=world.z-curvature_drop(world);
    var o:OceanOut;
    o.clip=to_clip(world);
    o.world=world;o.plane=plane;o.slope=grad;o.extra=vec2f(div,height+detail.z);
    return o;
}
fn wake(p:vec2f)->vec3f {
    // x: foam, yz: added surface gradient. A visual Kelvin pattern (19.47°
    // half angle, transverse wavelength 2πU²/g), not a hull-wave solution.
    var foam=0.;var grad=vec2f(0.);
    let count=min(u32(waves.header.w),8u);
    for(var i=0u;i<count;i=i+1u){
        let a=waves.wakes[i*2u];let b=waves.wakes[i*2u+1u];
        let speed=length(a.zw);
        if speed<0.3 {continue;}
        let dir=a.zw/speed;let side=vec2f(-dir.y,dir.x);
        let rel=p-a.xy;
        let along=-dot(rel,dir);
        let across=dot(rel,side);
        let length_=max(b.x,1.);let beam=max(b.y,0.5);
        let behind=along-length_*0.5;
        if behind< -length_*1.5 || behind>length_*40.+400. {continue;}
        let strength=clamp(speed/5.,0.,1.);
        let width=beam*0.55+max(behind,0.)*0.06;
        let wash=exp(-pow(across/width,2.))*exp(-max(behind,0.)/(10.*length_+40.))*smoothstep(-length_,-length_*0.3,behind);
        let arm_offset=(behind+length_)*0.3536;
        let arm=exp(-pow((abs(across)-arm_offset)/(beam*0.35+max(behind,0.)*0.015),2.))*exp(-max(behind,0.)/(18.*length_+80.))*step(-length_,behind);
        let bow=exp(-pow((along+length_*0.5)/(beam*0.9),2.)-pow(across/(beam*0.8),2.));
        foam=foam+(wash*0.9+arm*0.45+bow*0.7)*strength;
        let lambda=max(6.2832*speed*speed/9.81,0.5);
        let inside=smoothstep(arm_offset+beam,arm_offset-beam,abs(across))*step(0.,behind);
        let decay=exp(-max(behind,0.)/(8.*length_+60.));
        grad=grad+dir*cos(behind*6.2832/lambda)*0.12*inside*decay*strength;
        grad=grad+side*sign(across)*arm*0.15*strength;
    }
    return vec3f(clamp(foam,0.,1.),grad);
}
fn foam_texture(world:vec2f,t:f32)->f32 {
    // Foam is drawn out into streaks along the wind.
    let w=camera.sky.zw;
    let p=vec2f(dot(world,w)*0.45,dot(world,vec2f(-w.y,w.x)));
    // Lacy foam: large bubble cells broken by finer ones, domain-warped so
    // the cells read as streaks and patches rather than a regular pattern.
    let warp=vec2f(noise2(p*0.21+vec2f(t*0.03,0.)),noise2(p*0.21+vec2f(5.3,t*0.02)))*2.2;
    let a=cells(p*0.45+warp);
    let b=cells(p*1.6-warp*0.7+vec2f(0.,t*0.05));
    let c=cells(p*4.8+warp);
    return clamp(smoothstep(0.05,0.75,a)*0.55+smoothstep(0.1,0.7,b)*0.35+smoothstep(0.1,0.6,c)*0.2,0.,1.);
}
@fragment fn ocean_water(v:OceanOut,@builtin(front_facing) front:bool)->@location(0) vec4f {
    let t=camera.environment.x;
    let uv0=v.plane/camera.ocean.x;let uv1=v.plane/camera.ocean.y;let uv2=v.plane/camera.ocean.z;
    let s0=textureSample(ocean_slope,ocean_sampler,uv0,0);
    let s1=textureSample(ocean_slope,ocean_sampler,uv1,1);
    let s2=textureSample(ocean_slope,ocean_sampler,uv2,2);
    let m0=textureSample(ocean_moment,ocean_sampler,uv0,0);
    let m1=textureSample(ocean_moment,ocean_sampler,uv1,1);
    let m2=textureSample(ocean_moment,ocean_sampler,uv2,2);
    let d0=textureSample(ocean_disp,ocean_sampler,uv0,0);
    let d1=textureSample(ocean_disp,ocean_sampler,uv1,1);
    let d2=textureSample(ocean_disp,ocean_sampler,uv2,2);
    let on=camera.grid.w;
    let chop=camera.ocean.w;
    let jxx=(s0.z+s1.z+s2.z)*on;let jyy=(s0.w+s1.w+s2.w)*on;let jxy=(m0.z+m1.z+m2.z)*on;
    let wk=wake(v.plane);
    var grad=v.slope+(s0.xy+s1.xy+s2.xy)*on+wk.yz;
    grad=grad/max(vec2f(1.+chop*jxx+v.extra.x*0.5,1.+chop*jyy+v.extra.x*0.5),vec2f(0.3));
    let jacobian=(1.+chop*jxx+v.extra.x*0.5)*(1.+chop*jyy+v.extra.x*0.5)-chop*chop*jxy*jxy;
    let variance=(max(m0.x-s0.x*s0.x,0.)+max(m0.y-s0.y*s0.y,0.)+max(m1.x-s1.x*s1.x,0.)+max(m1.y-s1.y*s1.y,0.)
        +max(m2.x-s2.x*s2.x,0.)+max(m2.y-s2.y*s2.y,0.))*on+camera.foam.z;
    let up=normalize(vec3f(-grad,1.));
    let eye=camera.eye.xyz;
    let view=normalize(eye-v.world);
    let pixel=vec2i(clamp(v.clip.xy,vec2f(0.),camera.screen.xy-1.));
    let uv=v.clip.xy/camera.screen.xy;
    let distance=length(v.world-eye);
    // Foam: active whitecaps from the total Jacobian, calibrated to the
    // whitecap coverage of this sea, plus persistent foam from each cascade.
    let pattern=foam_texture(v.plane,t);
    // Breaking concentrates on the crests of the dominant waves and in wave
    // groups; the modulations average about one, keeping the coverage.
    let hs_now=max(waves.header.y,0.05);
    let crest_mod=mix(0.25,1.9,smoothstep(-0.1*hs_now,0.5*hs_now,v.extra.y));
    let groups=mix(0.35,1.65,smoothstep(0.3,0.7,fbm(v.plane*0.008+vec2f(t*0.004,0.),3)));
    let breaking=clamp((camera.foam.x-jacobian)/max(camera.foam.y,1e-4)*crest_mod*groups,0.,1.)*camera.foam.w;
    let persist=(d0.w*camera.foam_weights.x+d1.w*camera.foam_weights.y+d2.w*camera.foam_weights.z)*on*groups*mix(0.5,1.3,crest_mod*0.5);
    let far_fade=1.-smoothstep(2000.,9000.,distance)*0.6;
    var foam=clamp(max(breaking,persist*1.2)*(0.55+0.9*pattern)-(1.-pattern)*0.25,0.,1.)*far_fade;
    foam=max(foam,wk.x*(0.4+0.8*pattern));
    if camera.environment.z>0.5 {
        // Seen from below: Snell's window (refraction into the sky inside
        // ~48.6° of the vertical), total internal reflection outside.
        let incident=-view;
        let n=-up;
        let refracted=refract(incident,n,1.333);
        let cos_i=abs(dot(incident,n));
        let r0=0.02;
        var color:vec3f;
        let depth=max(-eye.z,0.);
        let below=deep_water(depth+3.)*1.1;
        if dot(refracted,refracted)<0.25 {
            color=below;
        } else {
            let f=r0+(1.-r0)*pow(1.-max(dot(refracted,-n),0.),5.);
            let sky=sky_full(refracted)+sun_disc(refracted)*0.25;
            color=sky*(1.-f)*light_at(depth)+below*f;
        }
        color=mix(color,below*0.7+camera.sun_light.rgb*max(camera.sun.z,0.)*0.08*light_at(depth),foam*0.6);
        return vec4f(underwater_fog(color,v.world),1.);
    }
    var n=up;
    let nv_raw=dot(n,view);
    if nv_raw<0.02 {n=normalize(n+view*(0.02-nv_raw));}
    let nv=max(dot(n,view),0.001);
    let fresnel=0.02+0.98*pow(1.-nv,5.);
    // Refraction of what lies below (seabed, hulls) through the water body.
    let depth_sample=textureLoad(scene_depth,pixel,0);
    let lin=linear_depth(depth_sample);
    let thickness=clamp(lin-dot(v.world-eye,camera.forward.xyz),0.,400.);
    var shifted=clamp(uv+n.xy*0.02*min(thickness,4.),vec2f(0.001),vec2f(0.999));
    let refracted_depth=textureLoad(scene_depth,vec2i(shifted*camera.screen.xy),0);
    if refracted_depth>v.clip.z {shifted=uv;}
    let behind=textureSample(scene_color,scene_sampler,shifted).rgb;
    let c=absorption()+scattering();
    let path=exp(-c*thickness);
    let body=behind*path+deep_water(0.)*(vec3f(1.)-path);
    // Light transmitted through thin crests towards a viewer facing the sun.
    let s=normalize(camera.sun.xyz);
    let hs=max(waves.header.y,0.1);
    let crest=clamp(v.extra.y/(hs*0.6)+0.35,0.,1.5);
    let toward=pow(max(dot(-view,normalize(vec3f(s.xy,-0.25*s.z))),0.),4.);
    let sss=vec3f(0.02,0.30,0.26)*camera.sun_light.rgb*(crest*toward*pow(0.5-0.5*dot(s,n),2.)*1.2+pow(nv,2.)*0.05)*0.25;
    // Sky reflection blurred by the unresolved slopes.
    var r=reflect(-view,n);
    r.z=abs(r.z);
    let alpha2=clamp(variance,0.0006,0.4);
    let reflected=mix(sky_full(r),camera.sky_light.rgb/PI,clamp(alpha2*3.,0.,0.6));
    // Sun glitter: Beckmann distribution of the surface slopes.
    let hv=normalize(s+view);
    let nh=max(dot(n,hv),0.001);
    let cos2=nh*nh;
    let tan2=(1.-cos2)/cos2;
    let glint=exp(-tan2/alpha2)/(PI*alpha2*cos2*cos2);
    let fh=0.02+0.98*pow(1.-max(dot(view,hv),0.),5.);
    let shadow=sun_visibility(v.world,vec3f(0.,0.,1.))*cloud_shadow(v.world);
    let spec=camera.sun_light.rgb*min(glint*fh/(4.*nv),300.)*shadow*step(0.,s.z)*(1.-foam);
    var color=mix(body+sss,reflected,fresnel)+spec;
    // Shoreline: thin water foams where swash runs up the beach.
    let shore=(1.-smoothstep(0.05,1.4,thickness))*(0.35+0.65*pattern)*(0.65+0.35*sin(t*0.9-thickness*2.5));
    foam=max(foam,shore*step(thickness,1.4)*0.9);
    let foam_light=camera.sun_light.rgb*max(s.z,0.)*shadow+camera.sky_light.rgb;
    color=mix(color,vec3f(0.9,0.93,0.94)*foam_light/PI*1.15,foam);
    return vec4f(aerial(color,v.world),1.);
}

// Legacy water (CPU-sampled or kind-5 mesh) for the offscreen render path.
@fragment fn water(v:Varying,@builtin(front_facing) front:bool)->@location(0) vec4f {
    var n=normalize(v.normal);if !front {n=-n;}
    let uv=v.clip.xy/camera.screen.xy;
    let pixel=vec2i(clamp(v.clip.xy,vec2f(0.),camera.screen.xy-1.));
    let depth=textureLoad(scene_depth,pixel,0);
    let lin=linear_depth(depth);
    let thickness=clamp(lin-dot(v.world-camera.eye.xyz,camera.forward.xyz),0.,80.);
    let shifted=clamp(uv+n.xy*0.018*min(thickness,5.),vec2f(0.001),vec2f(0.999));
    let transmitted=textureSample(scene_color,scene_sampler,shifted).rgb;
    let c=absorption()+scattering();
    let path=exp(-c*thickness);
    let water_color=transmitted*path+deep_water(0.)*(vec3f(1.)-path);
    let view=normalize(camera.eye.xyz-v.world);
    if camera.environment.z>0.5 {
        let refracted=refract(-view,-n,1.333);
        let below=deep_water(max(-camera.eye.z,0.)+3.);
        var color=below;
        if dot(refracted,refracted)>0.25 {color=sky_full(refracted)*light_at(max(-camera.eye.z,0.));}
        return vec4f(underwater_fog(color,v.world),1.);
    }
    var r=reflect(-view,n);r.z=abs(r.z);
    let fresnel=0.02+0.98*pow(1.-abs(dot(n,view)),5.);
    let sun=pow(max(dot(n,normalize(view+camera.sun.xyz)),0.),220.)*camera.sun_light.rgb;
    let shore=(1.-smoothstep(0.2,1.8,thickness))*0.45;
    var color=mix(water_color,sky_full(r),fresnel)+sun;
    color=mix(color,vec3f(0.8,0.9,0.88)*camera.sky_light.rgb,shore);
    return vec4f(aerial(color,v.world),1.);
}

// ------------------------------------------------------------ post
// Quarter-resolution bright pass for bloom (sun glints, sun disc, foam).
@fragment fn bloom_extract(v:Quad)->@location(0) vec4f {
    let px=1./vec2f(textureDimensions(scene_color));
    var c=vec3f(0.);
    for(var y=-1;y<=1;y=y+2){
        for(var x=-1;x<=1;x=x+2){
            c=c+textureSampleLevel(scene_color,scene_sampler,v.uv+vec2f(f32(x),f32(y))*px,0.).rgb;
        }
    }
    c=min(c*0.25,vec3f(400.));
    let ev=camera.environment.w*select(1.,2.4,camera.environment.z>0.5);
    let threshold=1.6/max(ev,0.01);
    let bright=max(c-vec3f(threshold),vec3f(0.));
    return vec4f(bright,1.);
}
fn bloom_at(uv:vec2f)->vec3f {
    let px=1./vec2f(textureDimensions(bloom_texture));
    var sum=vec3f(0.);var w=0.;
    for(var i=0;i<12;i=i+1){
        let a=f32(i)*0.5236;
        for(var r=1;r<=3;r=r+1){
            let rr=f32(r)*1.6;
            let weight=exp(-f32(r)*0.55);
            sum=sum+textureSampleLevel(bloom_texture,scene_sampler,uv+vec2f(cos(a),sin(a))*px*rr*f32(r),0.).rgb*weight;
            w=w+weight;
        }
    }
    return (sum/w+textureSampleLevel(bloom_texture,scene_sampler,uv,0.).rgb)*0.5;
}
fn aces(c:vec3f)->vec3f {
    return clamp((c*(2.51*c+0.03))/(c*(2.43*c+0.59)+0.14),vec3f(0.),vec3f(1.));
}
fn exposure()->f32 {return camera.environment.w*select(1.,2.4,camera.environment.z>0.5);}
fn tone_at(uv:vec2f)->vec3f {
    return aces(textureSampleLevel(scene_color,scene_sampler,uv,0.).rgb*exposure());
}
fn luma(c:vec3f)->f32 {return dot(c,vec3f(0.299,0.587,0.114));}
// Tone mapping, FXAA (console variant) and bloom added in HDR.
@fragment fn tone(v:Quad)->@location(0) vec4f {
    let px=1./vec2f(textureDimensions(scene_color));
    let hdr=textureSampleLevel(scene_color,scene_sampler,v.uv,0.).rgb;
    let m=aces(hdr*exposure());
    let nw=luma(tone_at(v.uv+vec2f(-1.,-1.)*px));
    let ne=luma(tone_at(v.uv+vec2f(1.,-1.)*px));
    let sw=luma(tone_at(v.uv+vec2f(-1.,1.)*px));
    let se=luma(tone_at(v.uv+vec2f(1.,1.)*px));
    let lm=luma(m);
    let lo=min(lm,min(min(nw,ne),min(sw,se)));
    let hi=max(lm,max(max(nw,ne),max(sw,se)));
    var dir=vec2f(-((nw+ne)-(sw+se)),(nw+sw)-(ne+se));
    let reduce=max((nw+ne+sw+se)*0.25*0.125,1./128.);
    dir=clamp(dir/(min(abs(dir.x),abs(dir.y))+reduce),vec2f(-8.),vec2f(8.))*px;
    let a=0.5*(tone_at(v.uv+dir*(1./3.-0.5))+tone_at(v.uv+dir*(2./3.-0.5)));
    let b=a*0.5+0.25*(tone_at(v.uv-dir*0.5)+tone_at(v.uv+dir*0.5));
    let lb=luma(b);
    let aa=select(b,a,lb<lo||lb>hi);
    var color=select(aa,m,hi-lo<max(0.0312,hi*0.125));
    let glow=bloom_at(v.uv);
    color=color+(aces((hdr+glow*0.35)*exposure())-m);
    let centre=v.uv-0.5;
    color=color*(1.-0.18*dot(centre,centre));
    return vec4f(clamp(color,vec3f(0.),vec3f(1.)),1.);
}
