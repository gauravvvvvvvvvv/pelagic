//! Repeatable CPU-only workload and bounded-allocation measurements.
//! Run release mode: acceptance.exe <new-report.json> [soak-ticks=36000].
use pelagic::{
    instruments::Suite,
    math::Vec3,
    render::{Renderer, SoftwareRenderer},
    scene::{Scene, world},
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};
struct Tracked;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
fn add(n: usize) {
    let live = LIVE.fetch_add(n, Ordering::Relaxed) + n;
    PEAK.fetch_max(live, Ordering::Relaxed);
}
// SAFETY: forwards the unchanged allocation layout/pointer to System; accounting
// does not allocate and is independent of allocation success.
unsafe impl GlobalAlloc for Tracked {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            add(l.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) };
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() {
            LIVE.fetch_sub(l.size(), Ordering::Relaxed);
            add(n);
        }
        q
    }
}
#[global_allocator]
static ALLOCATOR: Tracked = Tracked;
fn stats(mut v: Vec<f64>) -> serde_json::Value {
    v.sort_by(f64::total_cmp);
    serde_json::json!({"mean":v.iter().sum::<f64>()/v.len() as f64,"p95":v[((v.len() as f64*0.95).ceil() as usize-1).min(v.len()-1)],"max":v.last(),"n":v.len()})
}
fn timed(n: usize, mut f: impl FnMut()) -> serde_json::Value {
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        let t = Instant::now();
        f();
        v.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    stats(v)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let path = std::path::Path::new(args.get(1).ok_or("provide a new report filename")?);
    if path.exists() {
        return Err("report exists; choose a new filename".into());
    }
    let soak = args
        .get(2)
        .map(|s| s.parse::<u64>())
        .transpose()?
        .unwrap_or(36_000)
        .clamp(600, 216_000);
    let learning = Scene {
        school: pelagic::life::SchoolConfig {
            count: 0,
            ..Default::default()
        },
        particles: 0,
        ..Default::default()
    };
    let mut workloads = vec![];
    for (name, scene) in [
        ("learning", learning),
        ("vehicle", Scene::robotics_demo()),
        ("ecology", Scene::ecology_demo()),
    ] {
        let mut w = world(&scene)?;
        let mut suite = Suite::new(scene.sensors.clone());
        for _ in 0..120 {
            w.step();
            suite.update(&w, 1.0 / 60.0);
        }
        let step = timed(600, || w.step());
        let sampling = timed(1000, || {
            std::hint::black_box(w.ocean.velocity(Vec3::new(5.0, 3.0, -10.0), w.time()));
            std::hint::black_box(w.ocean.surface(5.0, 3.0, w.time()));
        });
        let snapshot = w.snapshot();
        let velocities: Vec<_> = w.vehicles().iter().map(|v| v.velocity).collect();
        let sensors = timed(100, || {
            std::hint::black_box(pelagic::instruments::sample(
                &w.ocean,
                &snapshot,
                &velocities,
                0,
                &scene.sensors,
            ));
        });
        let positions: Vec<_> = w.vehicles().iter().map(|v| v.position).collect();
        let mut schools = w.schools.clone();
        let behaviour = timed(300, || {
            for s in &mut schools {
                s.step(&w.ocean, &positions, 10.0, 1.0 / 60.0);
            }
        });
        let mut body_only = scene.clone();
        body_only.school.count = 0;
        body_only.particles = 0;
        let mut physical = world(&body_only)?;
        let physics = timed(600, || physical.step());
        let mut tiers = vec![];
        for (quality, width, height) in [(0, 512, 320), (1, 800, 500), (2, 1120, 700)] {
            let mut r = SoftwareRenderer::new(width, height)?;
            r.quality = quality;
            r.camera = Vec3::new(scene.camera[0], scene.camera[1], scene.camera[2]);
            r.target = Vec3::new(scene.target[0], scene.target[1], scene.target[2]);
            let t = Instant::now();
            std::hint::black_box(r.render(&w.ocean, &w.snapshot()));
            let cold = t.elapsed().as_secs_f64() * 1000.0;
            let render = timed(30, || {
                std::hint::black_box(r.render(&w.ocean, &w.snapshot()));
            });
            tiers.push(serde_json::json!({"quality":quality,"dimensions":[width,height],"cold_ms":cold,"render_ms":render}));
        }
        let encoded = scene.value().to_string();
        let parse = timed(50, || {
            let v = serde_json::from_str(&encoded).unwrap();
            std::hint::black_box(Scene::from_value(&v).unwrap());
        });
        // Warm pending sensor queues, then sample live allocated bytes without
        // retaining snapshots/timing arrays. This measures Rust heap, not RSS.
        for _ in 0..1200 {
            w.step();
            suite.update(&w, 1.0 / 60.0);
        }
        let before = LIVE.load(Ordering::Relaxed);
        let t = Instant::now();
        let mut finite = true;
        for tick in 0..soak {
            w.step();
            suite.update(&w, 1.0 / 60.0);
            if tick % 600 == 0 {
                finite &= w.checkpoint().validate().is_ok() && suite.validate().is_ok();
            }
        }
        let elapsed = t.elapsed().as_secs_f64();
        let after = LIVE.load(Ordering::Relaxed);
        let growth = after as i64 - before as i64;
        let passed = finite
            && growth < 2_097_152
            && step["p95"].as_f64().unwrap() < 5.0
            && tiers[0]["render_ms"]["p95"].as_f64().unwrap() < 33.334;
        workloads.push(serde_json::json!({"name":name,"bodies":w.vehicles().len(),"fish":scene.school.count,"tracers":scene.particles,"dt":1.0/60.0,"world_step_ms":step,"physics_only_ms":physics,"environment_query_pair_ms":sampling,"schooling_ms":behaviour,"sensor_sample_ms":sensors,"scene_parse_ms":parse,"tiers":tiers,"soak":{"ticks":soak,"simulated_seconds":soak as f64/60.0,"wall_seconds":elapsed,"live_heap_before":before,"live_heap_after":after,"heap_growth_bytes":growth,"finite":finite},"budget_pass":passed}));
    }
    let passed = workloads.iter().all(|w| w["budget_pass"] == true);
    let mut overload = Scene::robotics_demo();
    overload.school.count = 256;
    overload.particles = 512;
    overload.sensors.beams = 128;
    for id in 3..16 {
        let mut e = pelagic::entities::Entity::rov(id, Vec3::new(id as f64 * 2.0, 0.0, -12.0));
        e.model.hold_depth = Some(12.0);
        overload.entities.push(e);
    }
    let mut loaded = world(&overload)?;
    let heavy = timed(120, || loaded.step());
    loaded.checkpoint().validate()?;
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let run = pelagic::experiment::batch(&Scene::sensor_demo(), 5.0, &cancel, |_, _| {})?;
    let start = Instant::now();
    let mut bytes = vec![];
    run.write(&mut bytes)?;
    let encode = start.elapsed().as_secs_f64() * 1000.0;
    let parse_run = timed(5, || {
        std::hint::black_box(pelagic::experiment::Recording::read(bytes.as_slice()).unwrap());
    });
    let start = Instant::now();
    pelagic::experiment::verify(&run, &cancel)?;
    let verify = start.elapsed().as_secs_f64() * 1000.0;
    let folder = path.with_file_name(format!(
        "acceptance-io-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let start = Instant::now();
    pelagic::experiment::export_bundle(&folder, &run)?;
    let io = start.elapsed().as_secs_f64() * 1000.0;
    let report = serde_json::json!({"engine":env!("CARGO_PKG_VERSION"),"build":pelagic::BUILD_ID,"renderer":"CPU only","units":"milliseconds unless stated","rust_heap_peak_bytes":PEAK.load(Ordering::Relaxed),"workloads":workloads,"overload":{"bodies":16,"fish":256,"tracers":512,"world_step_ms":heavy,"finite":true,"policy":"supported bounds, reduced throughput; no accuracy/step-size substitution"},"io":{"samples":run.frames.len(),"bytes":bytes.len(),"encode_ms":encode,"parse_ms":parse_run,"verify_ms":verify,"dataset_write_ms":io},"pass":passed});
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    use std::io::Write;
    file.write_all(serde_json::to_string_pretty(&report)?.as_bytes())?;
    file.sync_all()?;
    if !passed {
        return Err("acceptance budget failed; inspect report".into());
    }
    Ok(())
}
