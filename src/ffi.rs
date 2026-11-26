//! Version 1 C ABI. Integer handles avoid cross-boundary Rust ownership/ABI assumptions.
use crate::{World, math::Vec3};
use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicU64, Ordering},
};
static WORLDS: OnceLock<Mutex<std::collections::BTreeMap<u64, World>>> = OnceLock::new();
static NEXT: AtomicU64 = AtomicU64::new(1);
fn worlds() -> &'static Mutex<std::collections::BTreeMap<u64, World>> {
    WORLDS.get_or_init(|| Mutex::new(Default::default()))
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct State {
    pub valid: i32,
    pub time: f64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub vx: f64,
    pub vy: f64,
    pub vz: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Sample {
    pub valid: i32,
    pub height: f64,
    pub bottom: f64,
    pub ux: f64,
    pub uy: f64,
    pub uz: f64,
    pub density: f64,
    pub sound_speed: f64,
}
#[unsafe(no_mangle)]
pub extern "C" fn pelagic_abi_version() -> u32 {
    1
}
#[unsafe(no_mangle)]
pub extern "C" fn pelagic_create(seed: u64) -> u64 {
    let scene = crate::scene::Scene {
        seed,
        ..Default::default()
    };
    let Ok(world) = crate::scene::world(&scene) else {
        return 0;
    };
    let Ok(mut map) = worlds().lock() else {
        return 0;
    };
    if map.len() >= 32 {
        return 0;
    }
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    map.insert(id, world);
    id
}
/// # Safety
/// `bytes` must reference `length` readable bytes for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pelagic_create_scene(bytes: *const u8, length: usize) -> u64 {
    if bytes.is_null() || length > 16_777_216 {
        return 0;
    }
    let input = unsafe { std::slice::from_raw_parts(bytes, length) };
    let Ok(value) = serde_json::from_slice(input) else {
        return 0;
    };
    let Ok(scene) = crate::scene::Scene::from_value(&value) else {
        return 0;
    };
    let Ok(world) = crate::scene::world(&scene) else {
        return 0;
    };
    let Ok(mut map) = worlds().lock() else {
        return 0;
    };
    if map.len() >= 32 {
        return 0;
    }
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    map.insert(id, world);
    id
}
#[unsafe(no_mangle)]
pub extern "C" fn pelagic_destroy(handle: u64) -> i32 {
    worlds()
        .lock()
        .ok()
        .and_then(|mut m| m.remove(&handle))
        .is_some() as i32
}
#[unsafe(no_mangle)]
pub extern "C" fn pelagic_step(handle: u64, ticks: u32) -> i32 {
    if ticks > 72000 {
        return 0;
    }
    let Ok(mut map) = worlds().lock() else {
        return 0;
    };
    let Some(world) = map.get_mut(&handle) else {
        return 0;
    };
    for _ in 0..ticks {
        world.step();
    }
    1
}
#[unsafe(no_mangle)]
pub extern "C" fn pelagic_command(
    handle: u64,
    entity: u64,
    fx: f64,
    fy: f64,
    fz: f64,
    tx: f64,
    ty: f64,
    tz: f64,
) -> i32 {
    if [fx, fy, fz, tx, ty, tz]
        .iter()
        .any(|v| !v.is_finite() || v.abs() > 1e6)
    {
        return 0;
    }
    let Ok(mut map) = worlds().lock() else {
        return 0;
    };
    let Some(world) = map.get_mut(&handle) else {
        return 0;
    };
    let Some(index) = world.vehicles().iter().position(|v| v.id == entity) else {
        return 0;
    };
    let body = world.vehicle_mut(index).unwrap();
    body.force = Vec3::new(fx, fy, fz);
    body.torque = Vec3::new(tx, ty, tz);
    1
}
#[unsafe(no_mangle)]
pub extern "C" fn pelagic_state(handle: u64, entity: u64) -> State {
    let Ok(map) = worlds().lock() else {
        return State::default();
    };
    let Some(world) = map.get(&handle) else {
        return State::default();
    };
    let Some(v) = world.vehicles().iter().find(|v| v.id == entity) else {
        return State::default();
    };
    State {
        valid: 1,
        time: world.time(),
        x: v.position.x,
        y: v.position.y,
        z: v.position.z,
        vx: v.velocity.x,
        vy: v.velocity.y,
        vz: v.velocity.z,
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn pelagic_sample(handle: u64, x: f64, y: f64, z: f64, time: f64) -> Sample {
    if [x, y, z, time].iter().any(|v| !v.is_finite()) {
        return Sample::default();
    }
    let Ok(map) = worlds().lock() else {
        return Sample::default();
    };
    let Some(w) = map.get(&handle) else {
        return Sample::default();
    };
    let p = Vec3::new(x, y, z);
    let u = w.ocean.velocity(p, time);
    let water = w.ocean.water(p);
    Sample {
        valid: 1,
        height: w.ocean.surface(x, y, time).height,
        bottom: w.ocean.bottom.elevation(x, y),
        ux: u.x,
        uy: u.y,
        uz: u.z,
        density: water.density,
        sound_speed: water.sound_speed,
    }
}
