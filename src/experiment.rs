//! Portable truth playback and observation records. Replay never advances physics.
use crate::{Snapshot, instruments::Observation, math::Vec3};
use std::io::{Read, Write};
pub const MAX_FRAMES: usize = 1200;
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RunFrame {
    pub truth: Snapshot,
    pub velocities: Vec<Vec3>,
    pub observations: Vec<Observation>,
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Recording {
    #[serde(default)]
    pub build_id: String,
    pub checkpoint: Option<crate::Checkpoint>,
    #[serde(default)]
    pub sensor_checkpoint: Option<crate::instruments::Suite>,
    pub commands: Vec<CommandEvent>,
    pub format: String,
    pub version: u32,
    pub engine: String,
    pub units: String,
    pub coordinates: String,
    pub scene_json: String,
    pub dt: f64,
    pub frames: Vec<RunFrame>,
}
impl Recording {
    pub fn new(scene_json: String, dt: f64) -> Self {
        Self {
            build_id: crate::BUILD_ID.into(),
            checkpoint: None,
            sensor_checkpoint: None,
            commands: vec![],
            format: "pelagic-run".into(),
            version: 1,
            engine: env!("CARGO_PKG_VERSION").into(),
            units: "SI".into(),
            coordinates: "X east; Y north; Z up; metres; seconds".into(),
            scene_json,
            dt,
            frames: vec![],
        }
    }
    pub fn append(&mut self, frame: RunFrame) -> Result<(), &'static str> {
        if self.frames.len() >= MAX_FRAMES {
            return Err("recording reached 1200 samples; save or export this run");
        }
        if self
            .frames
            .last()
            .is_some_and(|f| f.truth.tick >= frame.truth.tick)
        {
            return Err("run timestamps must strictly increase");
        }
        self.frames.push(frame);
        Ok(())
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(suite) = &self.sensor_checkpoint {
            suite.validate()?;
        }
        if let Some(state) = &self.checkpoint {
            state.validate()?;
            if state.dt != self.dt {
                return Err("checkpoint step differs from run");
            }
        }
        if self.commands.len() > 100000
            || self.commands.windows(2).any(|w| w[0].tick > w[1].tick)
            || self
                .commands
                .iter()
                .any(|c| !c.force.is_finite() || !c.torque.is_finite())
        {
            return Err("invalid command timeline");
        }
        if self.format != "pelagic-run"
            || self.version != 1
            || self.scene_json.len() > 16_777_216
            || self.frames.len() > MAX_FRAMES
            || !self.dt.is_finite()
            || self.dt <= 0.0
            || self.dt > 1.0
        {
            return Err("unsupported or oversized run");
        }
        let mut last = None;
        for f in &self.frames {
            let s = &f.truth;
            if s.vehicles.len() > 32
                || f.velocities.len() != s.vehicles.len()
                || s.fish.len() > 256
                || s.particles.len() > 512
                || f.observations.len() > 32
                || !s.time.is_finite()
                || (s.time - s.tick as f64 * self.dt).abs() > 1e-6
                || last.is_some_and(|t| s.tick <= t)
                || s.vehicles.iter().any(|v| {
                    !v.position.is_finite()
                        || !v.velocity.is_finite()
                        || !v.angular_velocity.is_finite()
                        || !v.half.is_finite()
                        || v.half.x <= 0.0
                        || v.half.y <= 0.0
                        || v.half.z <= 0.0
                        || [
                            v.orientation.w,
                            v.orientation.x,
                            v.orientation.y,
                            v.orientation.z,
                        ]
                        .iter()
                        .any(|x| !x.is_finite())
                })
                || s.fish.iter().any(|v| {
                    !v.position.is_finite()
                        || !v.velocity.is_finite()
                        || !v.phase.is_finite()
                        || !v.size.is_finite()
                })
                || s.particles
                    .iter()
                    .any(|p| !p.position.is_finite() || !p.age.is_finite())
                || f.velocities.iter().any(|v| !v.is_finite())
                || f.observations.iter().any(|o| {
                    o.beams.len() > 128
                        || !o.time.is_finite()
                        || !o.available_time.is_finite()
                        || o.available_time < o.time
                        || !o.velocity.is_finite()
                        || !o.angular_velocity.is_finite()
                        || !o.specific_force.is_finite()
                        || !o.position.is_finite()
                        || !o.depth.is_finite()
                        || o.beams.iter().any(|b| {
                            !b.angle.is_finite()
                                || !b.intensity.is_finite()
                                || b.range.is_some_and(|r| !r.is_finite() || r < 0.0)
                                || b.truth_range.is_some_and(|r| !r.is_finite() || r < 0.0)
                                || b.travel_seconds.is_some_and(|r| !r.is_finite() || r < 0.0)
                        })
                })
            {
                return Err("invalid run frame");
            }
            last = Some(s.tick);
        }
        Ok(())
    }
    pub fn capture_commands(&mut self, world: &crate::World) {
        for v in world.vehicles() {
            let previous = self.commands.iter().rev().find(|c| c.entity == v.id);
            if previous.is_none_or(|c| c.force != v.force || c.torque != v.torque)
                && self.commands.len() < 100000
            {
                self.commands.push(CommandEvent {
                    tick: world.tick(),
                    entity: v.id,
                    force: v.force,
                    torque: v.torque,
                });
            }
        }
    }
    pub fn write(&self, writer: impl Write) -> Result<(), String> {
        self.validate().map_err(str::to_string)?;
        let mut writer = std::io::BufWriter::new(writer);
        serde_json::to_writer(&mut writer, self).map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())
    }
    pub fn read(reader: impl Read) -> Result<Self, String> {
        let mut bytes = vec![];
        reader
            .take(67_108_865)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 67_108_864 {
            return Err("run exceeds 64 MB".into());
        }
        let run: Self = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        run.validate().map_err(str::to_string)?;
        Ok(run)
    }
    pub fn export_csv(&self, truth: impl Write, observations: impl Write) -> std::io::Result<()> {
        let mut truth = std::io::BufWriter::new(truth);
        let mut observations = std::io::BufWriter::new(observations);
        writeln!(
            truth,
            "tick,time_s,entity_id,x_m,y_m,z_m,vx_m_s,vy_m_s,vz_m_s,qw,qx,qy,qz"
        )?;
        writeln!(
            observations,
            "time_s,available_s,entity_id,beam,angle_rad,truth_range_m,measured_range_m,travel_s,intensity,target"
        )?;
        for frame in &self.frames {
            for (i, p) in frame.truth.vehicles.iter().enumerate() {
                let v = frame.velocities.get(i).copied().unwrap_or(Vec3::ZERO);
                let q = p.orientation;
                writeln!(
                    truth,
                    "{},{},{},{},{},{},{},{},{},{},{},{},{}",
                    frame.truth.tick,
                    frame.truth.time,
                    p.id,
                    p.position.x,
                    p.position.y,
                    p.position.z,
                    v.x,
                    v.y,
                    v.z,
                    q.w,
                    q.x,
                    q.y,
                    q.z
                )?;
            }
            for o in &frame.observations {
                for (i, b) in o.beams.iter().enumerate() {
                    let text = |v: Option<f64>| v.map_or(String::new(), |v| v.to_string());
                    writeln!(
                        observations,
                        "{},{},{},{},{},{},{},{},{},{}",
                        o.time,
                        o.available_time,
                        o.vehicle,
                        i,
                        b.angle,
                        text(b.truth_range),
                        text(b.range),
                        text(b.travel_seconds),
                        b.intensity,
                        b.target
                    )?;
                }
            }
        }
        truth.flush()?;
        observations.flush()?;
        Ok(())
    }
}
pub fn batch(
    scene: &crate::scene::Scene,
    seconds: f64,
    cancel: &std::sync::atomic::AtomicBool,
    mut progress: impl FnMut(u64, u64),
) -> Result<Recording, String> {
    if !seconds.is_finite() || !(0.1..=120.0).contains(&seconds) {
        return Err("batch duration must be 0.1–120 seconds".into());
    }
    let mut world = crate::scene::world(scene)?;
    let mut run = Recording::new(scene.value().to_string(), 1.0 / 60.0);
    run.checkpoint = Some(world.checkpoint());
    let ticks = (seconds / run.dt).round() as u64;
    let stride = scene.sensors.ticks(run.dt);
    let mut suite = crate::instruments::Suite::new(scene.sensors.clone());
    suite.update(&world, run.dt);
    run.sensor_checkpoint = Some(suite.clone());
    if ticks / stride + 1 > MAX_FRAMES as u64 {
        return Err("Duration and sample rate exceed 1200 samples. Reduce either setting.".into());
    }
    for tick in 0..=ticks {
        if tick > 0 {
            suite.update(&world, run.dt);
        }
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("Batch cancelled; completed previous runs are unchanged.".into());
        }
        if tick.is_multiple_of(stride) {
            let truth = world.snapshot();
            let velocities: Vec<_> = world.vehicles().iter().map(|v| v.velocity).collect();
            let observations = suite.latest.clone();
            run.append(RunFrame {
                truth,
                velocities,
                observations,
            })
            .map_err(str::to_string)?;
        }
        if tick.is_multiple_of(60) {
            progress(tick, ticks);
        }
        if tick < ticks {
            run.capture_commands(&world);
            world.step();
        }
    }
    run.validate().map_err(str::to_string)?;
    Ok(run)
}
/// Re-runs a recording's scene from its checkpoint, applying its recorded
/// commands, and calls `visit` at every recorded frame's tick with the
/// re-simulated world and sensor suite.
fn resimulate(
    run: &Recording,
    cancel: &std::sync::atomic::AtomicBool,
    mut visit: impl FnMut(
        &crate::World,
        Option<&crate::instruments::Suite>,
        &RunFrame,
    ) -> Result<(), String>,
) -> Result<(), String> {
    run.validate().map_err(str::to_string)?;
    let value = serde_json::from_str(&run.scene_json).map_err(|e| format!("run scene: {e}"))?;
    let scene = crate::scene::Scene::from_value(&value)?;
    let mut world = crate::scene::world(&scene)?;
    if let Some(state) = &run.checkpoint {
        world.restore(state).map_err(str::to_string)?;
    }
    let mut suite = run.sensor_checkpoint.clone();
    let mut command = 0;
    for frame in &run.frames {
        if frame.truth.tick > 7_200_000 {
            return Err("run exceeds verification time budget".into());
        }
        while world.tick() < frame.truth.tick {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err("Re-simulation cancelled".into());
            }
            while command < run.commands.len() && run.commands[command].tick <= world.tick() {
                let c = &run.commands[command];
                if let Some(i) = world.vehicles().iter().position(|v| v.id == c.entity) {
                    let v = world.vehicle_mut(i).unwrap();
                    v.force = c.force;
                    v.torque = c.torque;
                }
                command += 1;
            }
            world.step();
            if let Some(suite) = &mut suite {
                suite.update(&world, run.dt);
            }
        }
        visit(&world, suite.as_ref(), frame)?;
    }
    Ok(())
}
/// Checks that this build re-simulates every recorded truth sample exactly.
pub fn verify(run: &Recording, cancel: &std::sync::atomic::AtomicBool) -> Result<(), String> {
    resimulate(run, cancel, |world, suite, frame| {
        if world.snapshot() != frame.truth {
            return Err(format!(
                "Re-simulation diverges at tick {}. Playback remains exact; builds/platforms/models may differ.",
                frame.truth.tick
            ));
        }
        if let Some(suite) = suite
            && (suite.latest != frame.observations
                || world
                    .vehicles()
                    .iter()
                    .map(|v| v.velocity)
                    .collect::<Vec<_>>()
                    != frame.velocities)
        {
            return Err(format!(
                "Observation or velocity re-simulation diverges at tick {}",
                frame.truth.tick
            ));
        }
        Ok(())
    })
}
/// Re-records a run on this build: same scene, checkpoint, commands and
/// sample ticks, with truth and observations re-simulated by this engine.
/// Use it to migrate recordings made by an older build so they verify again.
pub fn rerecord(
    run: &Recording,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Recording, String> {
    let mut fresh = Recording {
        build_id: crate::BUILD_ID.into(),
        frames: Vec::with_capacity(run.frames.len()),
        ..run.clone()
    };
    fresh.engine = env!("CARGO_PKG_VERSION").into();
    resimulate(run, cancel, |world, suite, frame| {
        fresh.frames.push(RunFrame {
            truth: world.snapshot(),
            velocities: world.vehicles().iter().map(|v| v.velocity).collect(),
            observations: suite.map_or_else(|| frame.observations.clone(), |s| s.latest.clone()),
        });
        Ok(())
    })?;
    fresh.validate().map_err(str::to_string)?;
    Ok(fresh)
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CommandEvent {
    pub tick: u64,
    pub entity: u64,
    pub force: Vec3,
    pub torque: Vec3,
}
pub fn write_atomic(path: &std::path::Path, run: &Recording) -> Result<(), String> {
    write_atomic_with_replace(path, run, false)
}
/// Replacing an existing output is explicit; native Save As obtains that choice.
pub fn write_atomic_with_replace(
    path: &std::path::Path,
    run: &Recording,
    replace: bool,
) -> Result<(), String> {
    run.validate().map_err(str::to_string)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or("choose an output directory")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temp = parent.join(format!(
        ".pelagic-run-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        run.write(&mut file)?;
        file.sync_all().map_err(|e| e.to_string())?;
        if file.metadata().map_err(|e| e.to_string())?.len() > 67_108_864 {
            return Err("run exceeds 64 MB; export shorter recordings".into());
        }
        drop(file);
        if path.exists() && !replace {
            return Err("Run output already exists; choose a new filename.".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
            }
            let src: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
            let dst: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            if unsafe { MoveFileExW(src.as_ptr(), dst.as_ptr(), 8 | u32::from(replace)) } == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            Ok(())
        }
        #[cfg(not(windows))]
        {
            std::fs::rename(&temp, path).map_err(|e| e.to_string())
        }
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}
pub fn export_bundle(folder: &std::path::Path, run: &Recording) -> Result<(), String> {
    if folder.exists() {
        return Err("Export directory already exists; choose a new directory.".into());
    }
    run.validate().map_err(str::to_string)?;
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let result = (|| {
        write_atomic(&folder.join("experiment.pelagic-run"), run)?;
        run.export_csv(
            std::fs::File::create(folder.join("truth.csv")).map_err(|e| e.to_string())?,
            std::fs::File::create(folder.join("sonar.csv")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        std::fs::write(folder.join("scene.pelagic"), &run.scene_json).map_err(|e| e.to_string())?;
        export_auxiliary(folder, run).map_err(|e| e.to_string())?;
        let manifest = serde_json::json!({"format":"pelagic-dataset","version":1,"engine":run.engine,"engine_build":run.build_id,"units":run.units,"coordinates":run.coordinates,"dt":run.dt,"samples":run.frames.len(),"recording":"experiment.pelagic-run","scene":"scene.pelagic","truth":"truth.csv","observations":"sonar.csv","navigation":"navigation.csv","marine_life":"fish.csv","commands":"commands.csv","validation":"illustrative models; no field validation"});
        std::fs::write(
            folder.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::write(
            folder.join("INCOMPLETE.txt"),
            "Export did not finish; do not treat this directory as a complete dataset.",
        );
    }
    result
}
fn export_auxiliary(folder: &std::path::Path, run: &Recording) -> std::io::Result<()> {
    let mut nav = std::io::BufWriter::new(std::fs::File::create(folder.join("navigation.csv"))?);
    let mut fish = std::io::BufWriter::new(std::fs::File::create(folder.join("fish.csv"))?);
    let mut commands = std::io::BufWriter::new(std::fs::File::create(folder.join("commands.csv"))?);
    writeln!(
        nav,
        "sample_s,available_s,entity_id,x_m,y_m,z_m,depth_m,vx_m_s,vy_m_s,vz_m_s,gx_rad_s,gy_rad_s,gz_rad_s,ax_m_s2,ay_m_s2,az_m_s2"
    )?;
    writeln!(
        fish,
        "time_s,fish_id,x_m,y_m,z_m,swim_vx_m_s,swim_vy_m_s,swim_vz_m_s,phase_rad,size_m"
    )?;
    writeln!(
        commands,
        "tick,time_s,entity_id,fx_N,fy_N,fz_N,tx_Nm,ty_Nm,tz_Nm"
    )?;
    for f in &run.frames {
        for o in &f.observations {
            writeln!(
                nav,
                "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
                o.time,
                o.available_time,
                o.vehicle,
                o.position.x,
                o.position.y,
                o.position.z,
                o.depth,
                o.velocity.x,
                o.velocity.y,
                o.velocity.z,
                o.angular_velocity.x,
                o.angular_velocity.y,
                o.angular_velocity.z,
                o.specific_force.x,
                o.specific_force.y,
                o.specific_force.z
            )?;
        }
        for (id, v) in f.truth.fish.iter().enumerate() {
            writeln!(
                fish,
                "{},{},{},{},{},{},{},{},{},{}",
                f.truth.time,
                id,
                v.position.x,
                v.position.y,
                v.position.z,
                v.velocity.x,
                v.velocity.y,
                v.velocity.z,
                v.phase,
                v.size
            )?;
        }
    }
    for c in &run.commands {
        writeln!(
            commands,
            "{},{},{},{},{},{},{},{},{}",
            c.tick,
            c.tick as f64 * run.dt,
            c.entity,
            c.force.x,
            c.force.y,
            c.force.z,
            c.torque.x,
            c.torque.y,
            c.torque.z
        )?;
    }
    nav.flush()?;
    fish.flush()?;
    commands.flush()?;
    Ok(())
}
