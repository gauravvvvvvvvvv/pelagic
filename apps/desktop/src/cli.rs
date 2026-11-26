//! Headless command-line modes for automation and CI.
use pelagic::campaign::{self, Criteria, Variation};
use std::sync::atomic::AtomicBool;

pub const CAMPAIGN_USAGE: &str = "\
Usage: Pelagic.exe --campaign <project.pelagic> <report-folder> [options]
  --runs=N                 runs (1-500, default 20)
  --seconds=S              simulated seconds per run (0.1-120, default 30)
  --seed=B                 run i uses seed B+i (default 1)
  --wave-jitter=F          wave height scaled by 1 +/- F (0-0.9)
  --current-jitter=F       current speed scaled by 1 +/- F (0-0.9)
  --threads=N              worker threads (default: cores - 1, max 8)
Acceptance criteria (each optional):
  --max-nav-error=M        position fix error, metres
  --max-depth-excursion=M  depth change from start, metres
  --min-clearance=M        seabed clearance, metres
  --min-separation=M       closest approach between vehicles, metres
  --max-tilt=DEG           roll/pitch from upright, degrees
  --min-sonar-coverage=F   mean fraction of beams with a return (0-1)
Exit code: 0 all runs passed, 2 some runs failed, 1 invalid input or error.
Writes campaign.json, campaign.csv and summary.txt into the report folder.";

pub const TRIALS_USAGE: &str = "\
Usage: Pelagic.exe --trials <vehicle.pelagic-vehicle.json> <report-folder> [options]
  --only=LIST              comma list of decay,speed,turning,zigzag,stopping,waves
  --quick                  fewer speed levels, zig-zags and wave periods
  --scene=<project.pelagic> use the scene's density, depth, current and sea
  --water-depth=M          water depth for the basin (default 200)
  --depth=M                operating depth for submerged vehicles (default 20)
  --approach=F             approach thrust, fraction of capability (default 1)
  --rudder=DEG             turning-circle rudder angle (default 35)
  --yaw=F                  thruster-steered yaw command fraction (default 0.5)
  --wave-height=M          regular-wave height for wave response (default 0.5)
  --threads=N              worker threads (default: cores - 1, max 8)
Acceptance criteria (each optional):
  --imo                    IMO MSC.137(76) reference limits at the trial speed
  --min-speed=V            speed at full thrust, m/s
  --max-advance=L          advance, vehicle lengths
  --max-tactical=L         tactical diameter, vehicle lengths
  --max-overshoot=DEG      10/10 zig-zag first overshoot
  --max-stopping=L         stopping track reach, vehicle lengths
  --min-roll-damping=Z     roll damping ratio from free decay
  --max-roll-rao=R         roll response in waves, deg per deg of wave slope
Exit code: 0 all checks passed, 2 a check failed, 1 invalid input or error.
Writes trials.json, summary.txt and CSV tables into the report folder.";

/// Lets a GUI-subsystem executable print to the terminal that started it.
fn attach_console() {
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(
            windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
        );
    }
}

fn option<T: std::str::FromStr>(args: &[String], name: &str) -> Result<Option<T>, String> {
    let prefix = format!("--{name}=");
    match args.iter().find_map(|a| a.strip_prefix(&prefix)) {
        None => Ok(None),
        Some(v) => v
            .parse()
            .map(Some)
            .map_err(|_| format!("Invalid value for --{name}: {v}")),
    }
}

pub struct CampaignArgs {
    pub project: String,
    pub folder: String,
    pub seconds: f64,
    pub threads: usize,
    pub variation: Variation,
    pub criteria: Criteria,
}

pub fn parse_campaign(args: &[String]) -> Result<CampaignArgs, String> {
    let positional: Vec<&String> = args
        .iter()
        .skip(2)
        .filter(|a| !a.starts_with("--"))
        .collect();
    let [project, folder] = positional[..] else {
        return Err(CAMPAIGN_USAGE.into());
    };
    if let Some(unknown) = args.iter().skip(2).find(|a| {
        a.starts_with("--")
            && ![
                "runs",
                "seconds",
                "seed",
                "wave-jitter",
                "current-jitter",
                "threads",
                "max-nav-error",
                "max-depth-excursion",
                "min-clearance",
                "min-separation",
                "max-tilt",
                "min-sonar-coverage",
            ]
            .iter()
            .any(|known| a.starts_with(&format!("--{known}=")))
    }) {
        return Err(format!("Unknown option {unknown}\n\n{CAMPAIGN_USAGE}"));
    }
    let variation = Variation {
        runs: option(args, "runs")?.unwrap_or(20),
        seed_base: option(args, "seed")?.unwrap_or(1),
        wave_height_jitter: option(args, "wave-jitter")?.unwrap_or(0.0),
        current_jitter: option(args, "current-jitter")?.unwrap_or(0.0),
    };
    variation.validate()?;
    Ok(CampaignArgs {
        project: project.clone(),
        folder: folder.clone(),
        seconds: option(args, "seconds")?.unwrap_or(30.0),
        threads: option(args, "threads")?.unwrap_or_else(campaign::default_threads),
        variation,
        criteria: Criteria {
            max_nav_error_m: option(args, "max-nav-error")?,
            max_depth_excursion_m: option(args, "max-depth-excursion")?,
            min_seabed_clearance_m: option(args, "min-clearance")?,
            min_vehicle_separation_m: option(args, "min-separation")?,
            max_tilt_deg: option(args, "max-tilt")?,
            min_sonar_coverage: option(args, "min-sonar-coverage")?,
        },
    })
}

/// Plain-text summary shared by the console, `summary.txt` and the editor.
pub fn summary(report: &campaign::Report) -> String {
    let total = report.runs.len();
    let passed = report.passed();
    let errors = report.errors();
    let w = report.worst();
    let mut text = format!(
        "Campaign '{}': {passed}/{total} runs passed, {} failed, {errors} errors ({:.1} s each)\n\
         Worst case: nav error {:.3} m, depth excursion {:.3} m, seabed clearance {:.3} m, \
         vehicle separation {:.3} m, tilt {:.2}°, sonar coverage {:.2}\n",
        report.scene,
        total - passed - errors,
        report.seconds,
        w.nav_error_max_m,
        w.depth_excursion_max_m,
        w.seabed_clearance_min_m,
        w.vehicle_separation_min_m,
        w.tilt_max_deg,
        w.sonar_coverage_mean,
    );
    for r in report.runs.iter().filter(|r| !r.passed) {
        let why = r.error.clone().unwrap_or_else(|| r.failures.join("; "));
        text += &format!("  run {} (seed {}): {why}\n", r.index, r.seed);
    }
    text
}

pub struct TrialsArgs {
    pub vehicle: String,
    pub folder: String,
    pub scene: Option<String>,
    pub water_depth: Option<f64>,
    pub threads: usize,
    pub setup: pelagic::trials::Setup,
    pub criteria: pelagic::trials::Criteria,
    pub imo: bool,
}

pub fn parse_trials(args: &[String]) -> Result<TrialsArgs, String> {
    use pelagic::trials::{Criteria, Setup};
    let positional: Vec<&String> = args
        .iter()
        .skip(2)
        .filter(|a| !a.starts_with("--"))
        .collect();
    let [vehicle, folder] = positional[..] else {
        return Err(TRIALS_USAGE.into());
    };
    let known = [
        "only",
        "scene",
        "water-depth",
        "depth",
        "approach",
        "rudder",
        "yaw",
        "wave-height",
        "threads",
        "min-speed",
        "max-advance",
        "max-tactical",
        "max-overshoot",
        "max-stopping",
        "min-roll-damping",
        "max-roll-rao",
    ];
    if let Some(unknown) = args.iter().skip(2).find(|a| {
        a.starts_with("--")
            && *a != "--quick"
            && *a != "--imo"
            && !known.iter().any(|k| a.starts_with(&format!("--{k}=")))
    }) {
        return Err(format!("Unknown option {unknown}\n\n{TRIALS_USAGE}"));
    }
    let mut setup = if args.iter().any(|a| a == "--quick") {
        Setup::quick()
    } else {
        Setup::default()
    };
    if let Some(list) = option::<String>(args, "only")? {
        let wanted: Vec<&str> = list.split(',').map(str::trim).collect();
        for w in &wanted {
            if !["decay", "speed", "turning", "zigzag", "stopping", "waves"].contains(w) {
                return Err(format!("Unknown trial '{w}' in --only"));
            }
        }
        setup.free_decay = wanted.contains(&"decay");
        setup.speed = wanted.contains(&"speed");
        setup.turning = wanted.contains(&"turning");
        setup.zigzag = wanted.contains(&"zigzag");
        setup.stopping = wanted.contains(&"stopping");
        setup.waves = wanted.contains(&"waves");
    }
    if let Some(v) = option(args, "depth")? {
        setup.operating_depth = v;
    }
    if let Some(v) = option(args, "approach")? {
        setup.approach_thrust = v;
    }
    if let Some(v) = option(args, "rudder")? {
        setup.rudder_deg = v;
    }
    if let Some(v) = option(args, "yaw")? {
        setup.yaw_command = v;
    }
    if let Some(v) = option(args, "wave-height")? {
        setup.wave_height = v;
    }
    setup.validate()?;
    Ok(TrialsArgs {
        vehicle: vehicle.clone(),
        folder: folder.clone(),
        scene: option(args, "scene")?,
        water_depth: option(args, "water-depth")?,
        threads: option(args, "threads")?.unwrap_or_else(campaign::default_threads),
        setup,
        criteria: Criteria {
            min_speed_mps: option(args, "min-speed")?,
            max_advance_lengths: option(args, "max-advance")?,
            max_tactical_diameter_lengths: option(args, "max-tactical")?,
            max_first_overshoot_deg: option(args, "max-overshoot")?,
            max_stopping_track_lengths: option(args, "max-stopping")?,
            min_roll_damping: option(args, "min-roll-damping")?,
            max_roll_rao: option(args, "max-roll-rao")?,
            ..Criteria::default()
        },
        imo: args.iter().any(|a| a == "--imo"),
    })
}

/// `--trials`: standard trials for a vehicle definition. Returns the exit code.
pub fn trials(args: &[String]) -> i32 {
    attach_console();
    let parsed = match parse_trials(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let definition = match std::fs::read_to_string(&parsed.vehicle)
        .map_err(|e| e.to_string())
        .and_then(|t| pelagic::definition::VehicleDefinition::parse(&t))
    {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Could not load {}: {e}", parsed.vehicle);
            return 1;
        }
    };
    let mut environment = pelagic::trials::Environment::default();
    if let Some(path) = &parsed.scene {
        match crate::project::Document::open(std::path::Path::new(path)) {
            Ok(doc) => environment = pelagic::trials::Environment::from_scene(&doc.scene),
            Err(e) => {
                eprintln!("Could not open {path}: {e}");
                return 1;
            }
        }
    }
    if let Some(depth) = parsed.water_depth {
        environment.water_depth = depth;
    }
    let vehicle = definition.to_entity(1, pelagic::math::Vec3::ZERO, 0.0);
    let result = pelagic::trials::run(
        &vehicle,
        &environment,
        &parsed.setup,
        &parsed.criteria,
        parsed.threads,
        &AtomicBool::new(false),
        |done, total| eprint!("\rTrial {done}/{total}"),
    );
    eprintln!();
    let mut report = match result {
        Ok(report) => report,
        Err(e) => {
            eprintln!("Trials failed: {e}");
            return 1;
        }
    };
    if parsed.imo {
        let mut criteria = report.imo_criteria();
        let own = &parsed.criteria;
        criteria.min_speed_mps = own.min_speed_mps;
        criteria.min_roll_damping = own.min_roll_damping;
        criteria.max_roll_rao = own.max_roll_rao;
        report.judge_with(criteria);
    }
    if let Err(e) = report.write(std::path::Path::new(&parsed.folder)) {
        eprintln!("Could not write the report: {e}");
        return 1;
    }
    eprint!("{}", report.summary());
    if report.passed() { 0 } else { 2 }
}

pub const SEA_USAGE: &str = "\
Usage: Pelagic.exe --forecast <latitude> <longitude> <out-folder>
         7-day hourly sea forecast (wind sea, swell, current, wind, sea temperature)
       Pelagic.exe --seakeeping <vehicle.pelagic-vehicle.json> <report-folder> [options]
  --quick                  3 periods x 3 headings instead of 9 x 7
  --speed=F                transit thrust, fraction of capability (default 0.6)
  --criteria=NAME          naval | merchant | fast | survey (default naval)
  --forecast=LAT,LON       also judge the live forecast there (writes outlook.csv)
  --course=DEG             planned course for the outlook, degrees true (default 0)
  --threads=N              worker threads
Forecast data: Open-Meteo.com (CC BY 4.0); only the coordinates are sent.
Exit code: 0 done (seakeeping with a forecast: operable now), 2 not operable now, 1 error.";

/// `--forecast`: writes forecast.json and forecast.csv. Returns the exit code.
pub fn sea_forecast(args: &[String]) -> i32 {
    attach_console();
    let (Some(lat), Some(lon), Some(folder)) = (
        args.get(2).and_then(|v| v.parse::<f64>().ok()),
        args.get(3).and_then(|v| v.parse::<f64>().ok()),
        args.get(4),
    ) else {
        eprintln!("{SEA_USAGE}");
        return 1;
    };
    let forecast = match crate::forecast::point(lat, lon, 7) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Forecast failed: {e}");
            return 1;
        }
    };
    let folder = std::path::Path::new(folder);
    let mut csv = String::from(
        "time_utc,wave_height_m,wave_period_s,wave_from_deg,wind_wave_height_m,wind_wave_peak_period_s,swell_height_m,swell_peak_period_s,swell_from_deg,current_mps,current_towards_deg,sea_temperature_c,wind_mps,wind_from_deg\n",
    );
    let f = |v: Option<f64>| v.map_or(String::new(), |v| format!("{v:.3}"));
    for h in &forecast.hours {
        csv += &format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            h.time,
            f(h.wave_height),
            f(h.wave_period),
            f(h.wave_direction),
            f(h.wind_wave_height),
            f(h.wind_wave_peak_period),
            f(h.swell_height),
            f(h.swell_peak_period),
            f(h.swell_direction),
            f(h.current_speed),
            f(h.current_direction),
            f(h.sea_temperature),
            f(h.wind_speed),
            f(h.wind_direction)
        );
    }
    let written = std::fs::create_dir_all(folder)
        .and_then(|()| {
            std::fs::write(
                folder.join("forecast.json"),
                serde_json::to_vec_pretty(&forecast).unwrap_or_default(),
            )
        })
        .and_then(|()| std::fs::write(folder.join("forecast.csv"), csv));
    if let Err(e) = written {
        eprintln!("Could not write the forecast: {e}");
        return 1;
    }
    let now = forecast.index_at(&crate::forecast::now_hour());
    let h = &forecast.hours[now];
    eprintln!(
        "Forecast at {:.3}, {:.3} ({} hours). Now ({} UTC): Hs {} m, wind sea {} m, swell {} m, wind {} m/s, current {} m/s, SST {} °C",
        forecast.latitude,
        forecast.longitude,
        forecast.hours.len(),
        h.time,
        f(h.wave_height),
        f(h.wind_wave_height),
        f(h.swell_height),
        f(h.wind_speed),
        f(h.current_speed),
        f(h.sea_temperature)
    );
    eprintln!("{}", crate::forecast::ATTRIBUTION);
    0
}

/// `--seakeeping`: response table, limiting heights and optional outlook.
pub fn seakeeping(args: &[String]) -> i32 {
    use pelagic::seakeeping::{Criteria, Setup};
    attach_console();
    let positional: Vec<&String> = args
        .iter()
        .skip(2)
        .filter(|a| !a.starts_with("--"))
        .collect();
    let [vehicle, folder] = positional[..] else {
        eprintln!("{SEA_USAGE}");
        return 1;
    };
    let known = ["speed", "criteria", "forecast", "course", "threads"];
    if let Some(unknown) = args.iter().skip(2).find(|a| {
        a.starts_with("--")
            && *a != "--quick"
            && !known.iter().any(|k| a.starts_with(&format!("--{k}=")))
    }) {
        eprintln!("Unknown option {unknown}\n\n{SEA_USAGE}");
        return 1;
    }
    let parsed = (|| -> Result<_, String> {
        let mut setup = if args.iter().any(|a| a == "--quick") {
            Setup::quick()
        } else {
            Setup::default()
        };
        if let Some(v) = option(args, "speed")? {
            setup.speed_fraction = v;
        }
        setup.validate()?;
        let presets = Criteria::presets();
        let criteria = match option::<String>(args, "criteria")?.as_deref() {
            None | Some("naval") => presets[0].clone(),
            Some("merchant") => presets[1].clone(),
            Some("fast") => presets[2].clone(),
            Some("survey") => presets[3].clone(),
            Some(other) => return Err(format!("Unknown criteria '{other}'")),
        };
        let place = match option::<String>(args, "forecast")? {
            None => None,
            Some(text) => {
                let v: Vec<f64> = text
                    .split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();
                if v.len() != 2 {
                    return Err("--forecast needs LAT,LON".into());
                }
                Some((v[0], v[1]))
            }
        };
        let course: f64 = option(args, "course")?.unwrap_or(0.0);
        let threads = option(args, "threads")?.unwrap_or_else(campaign::default_threads);
        Ok((setup, criteria, place, course, threads))
    })();
    let (setup, criteria, place, course, threads) = match parsed {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let definition = match std::fs::read_to_string(vehicle)
        .map_err(|e| e.to_string())
        .and_then(|t| pelagic::definition::VehicleDefinition::parse(&t))
    {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Could not load {vehicle}: {e}");
            return 1;
        }
    };
    let report = match pelagic::seakeeping::run(
        &definition.to_entity(1, pelagic::math::Vec3::ZERO, 0.0),
        &pelagic::trials::Environment::default(),
        &setup,
        threads,
        &AtomicBool::new(false),
        |done, total| eprint!("\rSea state {done}/{total}"),
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("\nSeakeeping failed: {e}");
            return 1;
        }
    };
    eprintln!();
    let folder = std::path::Path::new(folder);
    if let Err(e) = report.write(folder, &criteria) {
        eprintln!("Could not write the report: {e}");
        return 1;
    }
    eprint!("{}", report.summary(&criteria));
    let Some((lat, lon)) = place else {
        return 0;
    };
    let forecast = match crate::forecast::point(lat, lon, 7) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Forecast failed: {e}");
            return 1;
        }
    };
    let outlook = report.outlook(&forecast, course, &criteria);
    let mut csv = String::from("time_utc,wave_height_m,operable,limited_by\n");
    for h in &outlook {
        csv += &format!(
            "{},{},{},\"{}\"\n",
            h.time,
            h.wave_height.map_or(String::new(), |v| format!("{v:.2}")),
            h.operable.map_or(String::new(), |v| v.to_string()),
            h.limited_by.join("; ")
        );
    }
    if let Err(e) = std::fs::write(folder.join("outlook.csv"), csv) {
        eprintln!("Could not write the outlook: {e}");
        return 1;
    }
    let judged: Vec<_> = outlook.iter().filter(|h| h.operable.is_some()).collect();
    let good = judged.iter().filter(|h| h.operable == Some(true)).count();
    let now = forecast.index_at(&crate::forecast::now_hour());
    eprintln!(
        "Outlook on course {course:.0}°: operable {good} of {} hours. Now: {}",
        judged.len(),
        match outlook[now].operable {
            Some(true) => "go".to_string(),
            Some(false) => format!("no-go ({})", outlook[now].limited_by.join(", ")),
            None => "no data".to_string(),
        }
    );
    eprintln!("{}", crate::forecast::ATTRIBUTION);
    if outlook[now].operable == Some(false) {
        2
    } else {
        0
    }
}

pub const SAR_USAGE: &str = "\
Usage: Pelagic.exe --sar <latitude> <longitude> <out-folder> [options]
       Pelagic.exe --sar --objects        (lists the search-object classes)
  Drift of a search object from its last known position (LKP) and the
  search plans with the best probability of success.
  --object=ID              leeway class (default PIW-1, person in water)
  --ago=H                  the object entered the water H hours ago (default 2)
  --window=H               ...or up to H hours after that (default 0.5)
  --radius=M               90 % radius of the LKP uncertainty (default 1000)
  --arrive=H               search unit on scene H hours from now (default 1)
  --endurance=H            time available for searching (default 4)
  --speed=KN               search speed, knots (default 10)
  --sweep=M                sweep width, metres (default 400)
  --definite               definite-range detection instead of the inverse cube law
  --fixed                  keep the pattern fixed instead of following the drift
  --pattern=P              SS, VS, PS or CS (default: all four)
  --particles=N            drift hypotheses (default 4000)
  --wind=SPEED,FROM        steady wind (m/s, from degrees) instead of the forecast
  --current=SPEED,TOWARDS  steady current (m/s, towards degrees), with --wind
  --seed=N  --threads=N
Writes summary.txt, drift.csv, particles.csv, plans.json, plan.gpx and
plan.waypoints (QGC WPL 110 mission for an uncrewed surface vehicle).
Forecast data: Open-Meteo.com (CC BY 4.0); only the coordinates are sent.
A planning aid: confirm plans with your rescue coordination centre's tools.
Exit code: 0 done, 1 error.";

pub struct SarArgs {
    pub folder: String,
    pub setup: pelagic::sar::DriftSetup,
    pub searcher: pelagic::sar::Searcher,
    /// Hours before now the object entered the water; search arrival from now.
    pub ago: f64,
    pub arrive: f64,
    pub patterns: Vec<pelagic::sar::Pattern>,
    /// Steady (wind speed, from, current speed, towards) instead of a forecast.
    pub steady: Option<[f64; 4]>,
    pub threads: usize,
}

fn pair(args: &[String], name: &str) -> Result<Option<[f64; 2]>, String> {
    let Some(text) = option::<String>(args, name)? else {
        return Ok(None);
    };
    let v: Vec<f64> = text
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    match v[..] {
        [a, b] if a.is_finite() && b.is_finite() && a >= 0.0 => Ok(Some([a, b])),
        _ => Err(format!("--{name} needs SPEED,DIRECTION")),
    }
}

pub fn parse_sar(args: &[String]) -> Result<SarArgs, String> {
    use pelagic::sar::{DriftSetup, Pattern, Searcher, find_object};
    let positional: Vec<&String> = args
        .iter()
        .skip(2)
        .filter(|a| !a.starts_with("--"))
        .collect();
    let [lat, lon, folder] = positional[..] else {
        return Err(SAR_USAGE.into());
    };
    let known = [
        "object",
        "ago",
        "window",
        "radius",
        "arrive",
        "endurance",
        "speed",
        "sweep",
        "pattern",
        "particles",
        "wind",
        "current",
        "seed",
        "threads",
    ];
    if let Some(unknown) = args.iter().skip(2).find(|a| {
        a.starts_with("--")
            && *a != "--definite"
            && *a != "--fixed"
            && !known.iter().any(|k| a.starts_with(&format!("--{k}=")))
    }) {
        return Err(format!("Unknown option {unknown}\n\n{SAR_USAGE}"));
    }
    let latitude: f64 = lat.parse().map_err(|_| format!("Invalid latitude {lat}"))?;
    let longitude: f64 = lon
        .parse()
        .map_err(|_| format!("Invalid longitude {lon}"))?;
    let mut setup = DriftSetup {
        latitude,
        longitude,
        ..Default::default()
    };
    if let Some(id) = option::<String>(args, "object")? {
        let o = find_object(&id).ok_or(format!(
            "Unknown object '{id}'. Run Pelagic.exe --sar --objects for the list."
        ))?;
        setup.object = o.id.into();
    }
    if let Some(v) = option(args, "window")? {
        setup.window_h = v;
    }
    if let Some(v) = option(args, "radius")? {
        setup.radius_m = v;
    }
    if let Some(v) = option(args, "particles")? {
        setup.particles = v;
    }
    if let Some(v) = option(args, "seed")? {
        setup.seed = v;
    }
    setup.validate()?;
    let mut searcher = Searcher {
        definite_range: args.iter().any(|a| a == "--definite"),
        follow_drift: !args.iter().any(|a| a == "--fixed"),
        ..Default::default()
    };
    if let Some(v) = option::<f64>(args, "speed")? {
        searcher.speed_mps = v * 0.514_444;
    }
    if let Some(v) = option(args, "sweep")? {
        searcher.sweep_width_m = v;
    }
    if let Some(v) = option(args, "endurance")? {
        searcher.endurance_h = v;
    }
    searcher.validate()?;
    let ago: f64 = option(args, "ago")?.unwrap_or(2.0);
    let arrive: f64 = option(args, "arrive")?.unwrap_or(1.0);
    if !(0.0..=46.0).contains(&ago) {
        return Err(
            "--ago must be between 0 and 46 hours (the forecast keeps two past days).".into(),
        );
    }
    if !(-ago..=48.0).contains(&arrive) {
        return Err("--arrive must be after the incident and within 48 hours.".into());
    }
    let patterns = match option::<String>(args, "pattern")? {
        None => vec![],
        Some(p) => {
            vec![Pattern::parse(&p).ok_or(format!("Unknown pattern '{p}' (SS, VS, PS, CS)"))?]
        }
    };
    let steady = match (pair(args, "wind")?, pair(args, "current")?) {
        (None, None) => None,
        (Some(w), c) => {
            let c = c.unwrap_or([0.0, 0.0]);
            Some([w[0], w[1], c[0], c[1]])
        }
        (None, Some(_)) => return Err("--current needs --wind as well.".into()),
    };
    Ok(SarArgs {
        folder: folder.clone(),
        setup,
        searcher,
        ago,
        arrive,
        patterns,
        steady,
        threads: option(args, "threads")?.unwrap_or_else(campaign::default_threads),
    })
}

/// `--sar`: drift, search plans and their probability of success.
pub fn sar(args: &[String]) -> i32 {
    use pelagic::sar::{Drivers, OBJECTS, run};
    attach_console();
    if args.iter().any(|a| a == "--objects") {
        for o in &OBJECTS {
            eprintln!(
                "{:<24} {}  (downwind {:.2} % of wind + {:.1} cm/s)",
                o.id, o.name, o.downwind[0], o.downwind[1]
            );
        }
        return 0;
    }
    let mut p = match parse_sar(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let now = crate::forecast::now_minute();
    let drivers = match p.steady {
        Some([ws, wf, cs, ct]) => {
            let start = pelagic::metocean::parse_utc(&now)
                .map(|t| pelagic::metocean::utc_hour(t - 48 * 3600))
                .unwrap_or_default();
            Drivers::steady(
                &start,
                48 + 72,
                p.setup.latitude,
                p.setup.longitude,
                ws,
                wf,
                cs,
                ct,
            )
        }
        None => {
            eprintln!(
                "Fetching wind and current around {:.3}, {:.3}…",
                p.setup.latitude, p.setup.longitude
            );
            match crate::forecast::drift_area(p.setup.latitude, p.setup.longitude)
                .and_then(|f| Drivers::from_forecasts(&f))
            {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Forecast failed: {e}");
                    return 1;
                }
            }
        }
    };
    let Some(now_hour) = drivers.hour_of(&now) else {
        eprintln!("Could not place the current time in the forecast.");
        return 1;
    };
    p.setup.incident_hour = now_hour - p.ago;
    p.searcher.start_hour = now_hour + p.arrive;
    let report = match run(
        &drivers,
        &p.setup,
        &p.searcher,
        &p.patterns,
        p.threads,
        &AtomicBool::new(false),
        |done, total| eprint!("\rPlan {done}/{total}"),
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("\nSearch planning failed: {e}");
            return 1;
        }
    };
    eprintln!();
    if let Err(e) = report.write(std::path::Path::new(&p.folder), 0) {
        eprintln!("Could not write the plan: {e}");
        return 1;
    }
    eprint!("{}", report.summary(0));
    if p.steady.is_none() {
        eprintln!("{}", crate::forecast::ATTRIBUTION);
    }
    0
}

pub const SURVEY_USAGE: &str = "\
Usage: Pelagic.exe --survey <project.pelagic | demo> <out-folder> [options]
  Survey lines over the scene's seabed, their coverage and IHO S-44 checks.
  --sensor=mbes|sss        multibeam (default) or side-scan
  --swath=DEG --range=M --beams=N --ping=HZ
  --underwater --altitude=M   seabed-following platform (default: surface)
  --draft=M                surface transducer depth (default 0.5)
  --order=2|1b|1a|special|exclusive   IHO S-44 Ed. 6.2.0 order (default 1a)
  --overlap=PCT --speed=KN --run-in=M
  --area=CX,CY,LENGTH,WIDTH   scene metres (default: the seabed, 10 % inside)
  --heading=DEG|contours   line direction (default 0 = north)
  --vehicle=NAME           also run the lines with this scene vehicle's physics
  --vehicle-file=PATH      ...or with a .pelagic-vehicle.json placed in the scene
Writes summary.txt, survey.json, lines.csv, coverage.asc, tvu.asc, depth.asc
and, for geo-referenced scenes, lines.gpx and survey.waypoints (QGC WPL 110).
Exit code: 0 meets the order's coverage, 2 short of it, 1 error.";

pub struct SurveyArgs {
    pub project: String,
    pub folder: String,
    pub setup: pelagic::survey::Setup,
    pub area: Option<[f64; 4]>,
    pub contours: bool,
    pub vehicle: Option<String>,
    pub vehicle_file: Option<String>,
}

pub fn parse_survey(args: &[String]) -> Result<SurveyArgs, String> {
    use pelagic::survey::{ORDERS, Setup, Sonar};
    let positional: Vec<&String> = args
        .iter()
        .skip(2)
        .filter(|a| !a.starts_with("--"))
        .collect();
    let [project, folder] = positional[..] else {
        return Err(SURVEY_USAGE.into());
    };
    let known = [
        "sensor",
        "swath",
        "range",
        "beams",
        "ping",
        "altitude",
        "draft",
        "order",
        "overlap",
        "speed",
        "run-in",
        "area",
        "heading",
        "vehicle",
        "vehicle-file",
    ];
    if let Some(unknown) = args.iter().skip(2).find(|a| {
        a.starts_with("--")
            && *a != "--underwater"
            && !known.iter().any(|k| a.starts_with(&format!("--{k}=")))
    }) {
        return Err(format!("Unknown option {unknown}\n\n{SURVEY_USAGE}"));
    }
    let mut setup = Setup::default();
    match option::<String>(args, "sensor")?.as_deref() {
        None | Some("mbes") | Some("multibeam") => {}
        Some("sss") | Some("side-scan") => {
            setup.sonar = Sonar::side_scan();
            setup.platform.underwater = true;
        }
        Some(other) => return Err(format!("Unknown sensor '{other}' (mbes or sss)")),
    }
    if let Some(v) = option(args, "swath")? {
        setup.sonar.swath_deg = v;
    }
    if let Some(v) = option(args, "range")? {
        setup.sonar.range_m = v;
    }
    if let Some(v) = option(args, "beams")? {
        setup.sonar.beams = v;
    }
    if let Some(v) = option(args, "ping")? {
        setup.sonar.ping_hz = v;
    }
    if args.iter().any(|a| a == "--underwater") {
        setup.platform.underwater = true;
    }
    if let Some(v) = option(args, "altitude")? {
        setup.platform.altitude_m = v;
    }
    if let Some(v) = option(args, "draft")? {
        setup.platform.draft_m = v;
    }
    if let Some(name) = option::<String>(args, "order")? {
        let key = name.to_ascii_lowercase();
        setup.order = ORDERS
            .iter()
            .position(|o| {
                let n = o.name.to_ascii_lowercase();
                n == format!("order {key}") || n.starts_with(&key)
            })
            .ok_or(format!(
                "Unknown order '{name}' (2, 1b, 1a, special, exclusive)"
            ))?;
    }
    if let Some(v) = option::<f64>(args, "overlap")? {
        setup.overlap = v / 100.0;
    }
    if let Some(v) = option::<f64>(args, "speed")? {
        setup.speed_mps = v * 0.514_444;
    }
    if let Some(v) = option(args, "run-in")? {
        setup.run_in_m = v;
    }
    let area = match option::<String>(args, "area")? {
        None => None,
        Some(text) => {
            let v: Vec<f64> = text
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();
            match v[..] {
                [x, y, l, w] => Some([x, y, l, w]),
                _ => return Err("--area needs CX,CY,LENGTH,WIDTH".into()),
            }
        }
    };
    if let Some([x, y, l, w]) = area {
        setup.area.centre = [x, y];
        setup.area.length_m = l;
        setup.area.width_m = w;
    }
    let mut contours = false;
    match option::<String>(args, "heading")?.as_deref() {
        None => {}
        Some("contours") => contours = true,
        Some(v) => {
            setup.area.axis_deg = v
                .parse::<f64>()
                .map_err(|_| format!("Invalid heading {v}"))?
                .rem_euclid(360.0)
        }
    }
    setup.validate()?;
    Ok(SurveyArgs {
        project: project.clone(),
        folder: folder.clone(),
        setup,
        area,
        contours,
        vehicle: option(args, "vehicle")?,
        vehicle_file: option(args, "vehicle-file")?,
    })
}

/// `--survey`: plan (and optionally run) a seabed survey.
pub fn survey(args: &[String]) -> i32 {
    use pelagic::survey;
    attach_console();
    let mut p = match parse_survey(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let scene = if p.project == "demo" {
        pelagic::scene::Scene::default()
    } else {
        match crate::project::Document::open(std::path::Path::new(&p.project)) {
            Ok(d) => d.scene,
            Err(e) => {
                eprintln!("Could not open {}: {e}", p.project);
                return 1;
            }
        }
    };
    let seabed = survey::seabed(&scene);
    if p.area.is_none() {
        let b = seabed.bounds();
        let a = &mut p.setup.area;
        a.centre = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
        let r = a.axis_deg.to_radians();
        let (w, h) = ((b[2] - b[0]) * 0.8, (b[3] - b[1]) * 0.8);
        let (along, across) = if r.sin().abs() > r.cos().abs() {
            (w, h)
        } else {
            (h, w)
        };
        a.length_m = along;
        a.width_m = across;
    }
    if p.contours {
        p.setup.area.axis_deg = survey::contour_heading(&seabed, &p.setup.area);
    }
    let vehicle = match (&p.vehicle, &p.vehicle_file) {
        (Some(name), _) => match scene.entities.iter().find(|e| &e.name == name) {
            Some(e) => Some(e.clone()),
            None if scene.vehicle && name == "Reference vehicle" => Some(scene.reference_entity()),
            None => {
                eprintln!("No vehicle named '{name}' in the scene.");
                return 1;
            }
        },
        (None, Some(path)) => match std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|t| pelagic::definition::VehicleDefinition::parse(&t))
        {
            Ok(d) => Some(d.to_entity(1, pelagic::math::Vec3::ZERO, 0.0)),
            Err(e) => {
                eprintln!("Could not load {path}: {e}");
                return 1;
            }
        },
        (None, None) => None,
    };
    let report = match survey::run(
        &scene,
        &p.setup,
        vehicle.as_ref(),
        &AtomicBool::new(false),
        |f| eprint!("\rRunning the lines: {:.0} %", f * 100.0),
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("\nSurvey failed: {e}");
            return 1;
        }
    };
    if vehicle.is_some() {
        eprintln!();
    }
    if let Err(e) = report.write(std::path::Path::new(&p.folder)) {
        eprintln!("Could not write the survey: {e}");
        return 1;
    }
    eprint!("{}", report.summary());
    let judged = report.simulated.as_ref().unwrap_or(&report.ideal);
    if judged.stats.meets_order { 0 } else { 2 }
}

pub const COLREGS_USAGE: &str = "\
Usage: Pelagic.exe --colregs <vehicle.pelagic-vehicle.json> <out-folder> [options]
  Runs the standard collision-regulations encounters (head-on, crossing,
  overtaking, Rule 17 and Rule 18 cases, two vessels at once) and random
  ones with the reference COLREGs autopilot on the vehicle's physics.
  --random=N               random encounters (default 12)
  --seed=N                 first random seed (default 1)
  --safe=M                 safe passing distance (default 15 vessel lengths)
  --only=standard|random   one set only
  --threads=N
Writes summary.txt, colregs.json and track-<encounter>.csv files.
Thresholds are training interpretations; the Regulations give no numbers.
Exit code: 0 every check passed, 2 some failed, 1 error.";

pub struct ColregsArgs {
    pub vehicle: String,
    pub folder: String,
    pub random: u64,
    pub seed: u64,
    pub safe: Option<f64>,
    pub standard: bool,
    pub threads: usize,
}

pub fn parse_colregs(args: &[String]) -> Result<ColregsArgs, String> {
    let positional: Vec<&String> = args
        .iter()
        .skip(2)
        .filter(|a| !a.starts_with("--"))
        .collect();
    let [vehicle, folder] = positional[..] else {
        return Err(COLREGS_USAGE.into());
    };
    let known = ["random", "seed", "safe", "only", "threads"];
    if let Some(unknown) = args
        .iter()
        .skip(2)
        .find(|a| a.starts_with("--") && !known.iter().any(|k| a.starts_with(&format!("--{k}="))))
    {
        return Err(format!("Unknown option {unknown}\n\n{COLREGS_USAGE}"));
    }
    let mut random: u64 = option(args, "random")?.unwrap_or(12);
    let mut standard = true;
    match option::<String>(args, "only")?.as_deref() {
        None => {}
        Some("standard") => random = 0,
        Some("random") => standard = false,
        Some(other) => return Err(format!("--only must be standard or random, not {other}")),
    }
    if random > 500 {
        return Err("Use at most 500 random encounters.".into());
    }
    let safe: Option<f64> = option(args, "safe")?;
    if safe.is_some_and(|s| !(5.0..=20_000.0).contains(&s)) {
        return Err("--safe must be between 5 and 20000 m.".into());
    }
    if !standard && random == 0 {
        return Err("Nothing to run.".into());
    }
    Ok(ColregsArgs {
        vehicle: vehicle.clone(),
        folder: folder.clone(),
        random,
        seed: option(args, "seed")?.unwrap_or(1),
        safe,
        standard,
        threads: option(args, "threads")?.unwrap_or_else(campaign::default_threads),
    })
}

/// `--colregs`: the encounter suite with the reference autopilot.
pub fn colregs(args: &[String]) -> i32 {
    use pelagic::colregs::{self, Rules};
    attach_console();
    let p = match parse_colregs(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let vehicle = match std::fs::read_to_string(&p.vehicle)
        .map_err(|e| e.to_string())
        .and_then(|t| pelagic::definition::VehicleDefinition::parse(&t))
    {
        Ok(d) => d.to_entity(1, pelagic::math::Vec3::ZERO, 0.0),
        Err(e) => {
            eprintln!("Could not load {}: {e}", p.vehicle);
            return 1;
        }
    };
    let scene = pelagic::scene::Scene {
        structure: false,
        ..Default::default()
    };
    let (speed, length) = match colregs::service_speed(&scene, &vehicle) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let mut rules = Rules::for_length(length);
    if let Some(safe) = p.safe {
        let k = safe / rules.safe_cpa_m;
        rules.safe_cpa_m *= k;
        rules.early_range_m *= k;
        rules.may_act_range_m *= k;
        rules.extremis_range_m *= k;
    }
    let mut scenarios = if p.standard {
        colregs::standard(speed, length, &rules)
    } else {
        vec![]
    };
    for seed in p.seed..p.seed + p.random {
        scenarios.push(colregs::random(speed, length, &rules, seed));
    }
    eprintln!(
        "{}: {:.1} kn service speed, safe passing distance {:.0} m, {} encounters",
        vehicle.name,
        speed / 0.514_444,
        rules.safe_cpa_m,
        scenarios.len()
    );
    let results = match colregs::run_suite(
        &scene,
        &vehicle,
        &scenarios,
        &rules,
        p.threads,
        &AtomicBool::new(false),
        |done, total| eprint!("\rEncounter {done}/{total}"),
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("\nEncounters failed: {e}");
            return 1;
        }
    };
    eprintln!();
    if let Err(e) = colregs::write_suite(std::path::Path::new(&p.folder), &results) {
        eprintln!("Could not write the results: {e}");
        return 1;
    }
    let passed = results.iter().filter(|(a, _)| a.passed).count();
    for (a, _) in &results {
        if !a.passed {
            eprint!("{}", a.summary());
        }
    }
    eprintln!(
        "{passed} of {} encounters passed every check.",
        results.len()
    );
    if passed == results.len() { 0 } else { 2 }
}

pub const SPILL_USAGE: &str = "\
Usage: Pelagic.exe --spill <latitude> <longitude> <out-folder> [options]
  Oil spill drift and evaporation from a release point.
  --oil=ID                 ANS, ASMB, ARAB-LIGHT, ARAB-MEDIUM, ARAB-HEAVY or CUSTOM
  --distilled=PCT          with --oil=CUSTOM: percentage distilled at 180 °C
  --volume=M3              released volume (default 100)
  --ago=H                  release began H hours ago (default 0; negative = future)
  --release=H              continuous release over H hours (default 0 = at once)
  --hours=H                follow the oil for H hours (default 48)
  --wind=SPEED,FROM        steady wind (m/s, from °) instead of the forecast
  --current=SPEED,TOWARDS  steady current (m/s, towards °), with --wind
  --wind-factor=PCT        wind drift (default 3 % of the 10 m wind)
  --deflection=DEG         wind drift deflected to the right (default 0)
  --diffusivity=M2S        horizontal diffusivity (default 10 m²/s)
  --temp=C                 water temperature (default 20 °C)
  --project=PATH           strand on this real-world project's coastline
  --particles=N --seed=N
Writes summary.txt, mass_balance.csv, particles.csv and slick.geojson.
Exit code: 0 no oil ashore, 2 oil ashore, 1 error.";

pub struct SpillArgs {
    pub setup: pelagic::spill::SpillSetup,
    pub ago: f64,
    pub steady: Option<[f64; 4]>,
    pub project: Option<String>,
    pub folder: String,
}

pub fn parse_spill(args: &[String]) -> Result<SpillArgs, String> {
    let positional: Vec<&String> = args
        .iter()
        .skip(2)
        .filter(|a| !a.starts_with("--") || a.parse::<f64>().is_ok())
        .collect();
    let [lat, lon, folder] = positional[..] else {
        return Err(SPILL_USAGE.into());
    };
    let known = [
        "oil",
        "distilled",
        "volume",
        "ago",
        "release",
        "hours",
        "wind",
        "current",
        "wind-factor",
        "deflection",
        "diffusivity",
        "temp",
        "project",
        "particles",
        "seed",
    ];
    if let Some(unknown) = args.iter().skip(2).find(|a| {
        a.starts_with("--")
            && a.parse::<f64>().is_err()
            && !known.iter().any(|k| a.starts_with(&format!("--{k}=")))
    }) {
        return Err(format!("Unknown option {unknown}\n\n{SPILL_USAGE}"));
    }
    let mut s = pelagic::spill::SpillSetup {
        latitude: lat.parse().map_err(|_| format!("Invalid latitude {lat}"))?,
        longitude: lon
            .parse()
            .map_err(|_| format!("Invalid longitude {lon}"))?,
        ..Default::default()
    };
    if let Some(oil) = option::<String>(args, "oil")? {
        s.product = oil.to_ascii_uppercase();
    }
    if let Some(v) = option(args, "distilled")? {
        s.percent_distilled = v;
    }
    if let Some(v) = option(args, "volume")? {
        s.volume_m3 = v;
    }
    if let Some(v) = option(args, "release")? {
        s.release_h = v;
    }
    if let Some(v) = option(args, "hours")? {
        s.duration_h = v;
    }
    if let Some(v) = option::<f64>(args, "wind-factor")? {
        s.wind_factor = v / 100.0;
    }
    if let Some(v) = option(args, "deflection")? {
        s.deflection_deg = v;
    }
    if let Some(v) = option(args, "diffusivity")? {
        s.diffusivity = v;
    }
    if let Some(v) = option(args, "temp")? {
        s.water_temp_c = v;
    }
    if let Some(v) = option(args, "particles")? {
        s.particles = v;
    }
    if let Some(v) = option(args, "seed")? {
        s.seed = v;
    }
    s.validate()?;
    let ago: f64 = option(args, "ago")?.unwrap_or(0.0);
    if !(-48.0..=46.0).contains(&ago) {
        return Err("--ago must be between -48 and 46 hours.".into());
    }
    let steady = match (pair(args, "wind")?, pair(args, "current")?) {
        (None, None) => None,
        (Some(w), c) => {
            let c = c.unwrap_or([0.0, 0.0]);
            Some([w[0], w[1], c[0], c[1]])
        }
        (None, Some(_)) => return Err("--current needs --wind as well.".into()),
    };
    Ok(SpillArgs {
        setup: s,
        ago,
        steady,
        project: option(args, "project")?,
        folder: folder.clone(),
    })
}

/// `--spill`: oil spill drift.
pub fn spill(args: &[String]) -> i32 {
    use pelagic::sar::{Drivers, Plane};
    attach_console();
    let SpillArgs {
        mut setup,
        ago,
        steady,
        project,
        folder,
    } = match parse_spill(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let land = match &project {
        None => None,
        Some(path) => match crate::project::Document::open(std::path::Path::new(path)) {
            Ok(d) => {
                let o = &d.scene.provenance;
                Some(pelagic::spill::Land {
                    grid: pelagic::survey::seabed(&d.scene),
                    offset: Plane {
                        latitude: o.origin_latitude,
                        longitude: o.origin_longitude,
                    }
                    .local(setup.latitude, setup.longitude),
                })
            }
            Err(e) => {
                eprintln!("Could not open {path}: {e}");
                return 1;
            }
        },
    };
    let now = crate::forecast::now_minute();
    let drivers = match steady {
        Some([ws, wf, cs, ct]) => {
            let start = pelagic::metocean::parse_utc(&now)
                .map(|t| pelagic::metocean::utc_hour(t - 48 * 3600))
                .unwrap_or_default();
            Drivers::steady(&start, 120, setup.latitude, setup.longitude, ws, wf, cs, ct)
        }
        None => {
            eprintln!(
                "Fetching wind and current around {:.3}, {:.3}…",
                setup.latitude, setup.longitude
            );
            match crate::forecast::drift_area(setup.latitude, setup.longitude)
                .and_then(|f| Drivers::from_forecasts(&f))
            {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Forecast failed: {e}");
                    return 1;
                }
            }
        }
    };
    let Some(now_h) = drivers.hour_of(&now) else {
        eprintln!("Could not place the current time in the forecast.");
        return 1;
    };
    setup.release_hour = now_h - ago;
    let room = (drivers.hours - 1) as f64 - setup.release_hour;
    if room < 0.5 {
        eprintln!("The release is outside the forecast period.");
        return 1;
    }
    if setup.duration_h > room {
        eprintln!("Following for {room:.1} h (the end of the forecast).");
        setup.duration_h = room;
    }
    let s = match pelagic::spill::run(&drivers, &setup, land.as_ref(), &AtomicBool::new(false)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Spill drift failed: {e}");
            return 1;
        }
    };
    if let Err(e) = s.write(std::path::Path::new(&folder)) {
        eprintln!("Could not write the results: {e}");
        return 1;
    }
    eprint!("{}", s.summary());
    if steady.is_none() {
        eprintln!("{}", crate::forecast::ATTRIBUTION);
    }
    if s.first_landfall().is_some() { 2 } else { 0 }
}

pub const INSPECT_USAGE: &str = "\
Usage: Pelagic.exe --inspect <project.pelagic | demo> <out-folder> [options]
  Camera passes over harbour pilings or a vessel's hull and their coverage.
  --target=pilings|NAME    the harbour pilings (default) or a vessel's hull
  --vehicle=NAME           also fly it with this scene vehicle (an ROV)
  --vehicle-file=PATH      ...or with a .pelagic-vehicle.json
  --gsd=MM                 required sharpness, mm per pixel (default 1)
  --fov=DEG --pixels=WxH   camera (default 70°, 1920x1080)
  --overlap=PCT            between passes (default 30)
  --contrast=PCT           least image contrast (default 25)
  --attenuation=PER_M      water beam attenuation (default: the scene's, at least 0.05)
  --speed=MPS              along passes (default 0.25)
  --standoff=M             fixed stand-off (default: from sharpness and clarity)
Writes summary.txt, inspection.json, faces.csv, path.csv and track.csv.
Exit code: 0 at least 95 % imaged to spec, 2 less, 1 error.";

pub struct InspectArgs {
    pub project: String,
    pub folder: String,
    pub target: String,
    pub vehicle: Option<String>,
    pub vehicle_file: Option<String>,
    pub setup: pelagic::inspection::Setup,
    pub attenuation: Option<f64>,
}

pub fn parse_inspect(args: &[String]) -> Result<InspectArgs, String> {
    let positional: Vec<&String> = args
        .iter()
        .skip(2)
        .filter(|a| !a.starts_with("--"))
        .collect();
    let [project, folder] = positional[..] else {
        return Err(INSPECT_USAGE.into());
    };
    let known = [
        "target",
        "vehicle",
        "vehicle-file",
        "gsd",
        "fov",
        "pixels",
        "overlap",
        "contrast",
        "attenuation",
        "speed",
        "standoff",
    ];
    if let Some(unknown) = args
        .iter()
        .skip(2)
        .find(|a| a.starts_with("--") && !known.iter().any(|k| a.starts_with(&format!("--{k}="))))
    {
        return Err(format!("Unknown option {unknown}\n\n{INSPECT_USAGE}"));
    }
    let mut s = pelagic::inspection::Setup::default();
    if let Some(v) = option(args, "gsd")? {
        s.gsd_mm = v;
    }
    if let Some(v) = option(args, "fov")? {
        s.camera.fov_deg = v;
    }
    if let Some(text) = option::<String>(args, "pixels")? {
        let v: Vec<f64> = text
            .split(['x', 'X'])
            .filter_map(|p| p.parse().ok())
            .collect();
        match v[..] {
            [w, h] => s.camera.pixels = [w, h],
            _ => return Err("--pixels needs WIDTHxHEIGHT".into()),
        }
    }
    if let Some(v) = option::<f64>(args, "overlap")? {
        s.overlap = v / 100.0;
    }
    if let Some(v) = option::<f64>(args, "contrast")? {
        s.min_contrast = v / 100.0;
    }
    let attenuation: Option<f64> = option(args, "attenuation")?;
    if let Some(v) = attenuation {
        s.attenuation = v;
    }
    if let Some(v) = option(args, "speed")? {
        s.speed_mps = v;
    }
    if let Some(v) = option(args, "standoff")? {
        s.standoff_m = v;
    }
    s.validate()?;
    Ok(InspectArgs {
        project: project.clone(),
        folder: folder.clone(),
        target: option(args, "target")?.unwrap_or_else(|| "pilings".into()),
        vehicle: option(args, "vehicle")?,
        vehicle_file: option(args, "vehicle-file")?,
        setup: s,
        attenuation,
    })
}

/// `--inspect`: inspection plan and optional flight.
pub fn inspect(args: &[String]) -> i32 {
    use pelagic::inspection;
    attach_console();
    let mut p = match parse_inspect(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let mut scene = if p.project == "demo" {
        pelagic::scene::Scene::default()
    } else {
        match crate::project::Document::open(std::path::Path::new(&p.project)) {
            Ok(d) => d.scene,
            Err(e) => {
                eprintln!("Could not open {}: {e}", p.project);
                return 1;
            }
        }
    };
    if p.attenuation.is_none() {
        p.setup.attenuation = scene.attenuation.max(0.05);
    }
    let target = if p.target == "pilings" {
        inspection::pilings(&scene)
    } else {
        match scene.entities.iter().find(|e| e.name == p.target) {
            Some(e) => {
                let t = inspection::hull(e, scene.density);
                let id = e.id;
                scene.entities.retain(|x| x.id != id);
                t
            }
            None => Err(format!("No vessel named '{}' in the scene.", p.target)),
        }
    };
    let target = match target {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let vehicle = match (&p.vehicle, &p.vehicle_file) {
        (Some(name), _) => match scene.entities.iter().find(|e| &e.name == name) {
            Some(e) => Some(e.clone()),
            None if scene.vehicle && name == "Reference vehicle" => Some(scene.reference_entity()),
            None => {
                eprintln!("No vehicle named '{name}' in the scene.");
                return 1;
            }
        },
        (None, Some(path)) => match std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|t| pelagic::definition::VehicleDefinition::parse(&t))
        {
            Ok(d) => Some(d.to_entity(1, pelagic::math::Vec3::ZERO, 0.0)),
            Err(e) => {
                eprintln!("Could not load {path}: {e}");
                return 1;
            }
        },
        (None, None) => None,
    };
    let report = match inspection::run(
        &scene,
        target,
        &p.setup,
        vehicle.as_ref(),
        &AtomicBool::new(false),
        |f| eprint!("\rFlying the passes: {:.0} %", f * 100.0),
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("\nInspection failed: {e}");
            return 1;
        }
    };
    if vehicle.is_some() {
        eprintln!();
    }
    if let Err(e) = report.write(std::path::Path::new(&p.folder)) {
        eprintln!("Could not write the results: {e}");
        return 1;
    }
    eprint!("{}", report.summary());
    let judged = report.simulated.as_ref().unwrap_or(&report.ideal);
    if judged.covered_percent >= 95.0 { 0 } else { 2 }
}

/// `--campaign`: returns the process exit code.
pub fn campaign(args: &[String]) -> i32 {
    attach_console();
    let parsed = match parse_campaign(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let doc = match crate::project::Document::open(std::path::Path::new(&parsed.project)) {
        Ok(doc) => doc,
        Err(e) => {
            eprintln!("Could not open {}: {e}", parsed.project);
            return 1;
        }
    };
    let result = campaign::run(
        &doc.scene,
        parsed.seconds,
        &parsed.variation,
        &parsed.criteria,
        parsed.threads,
        &AtomicBool::new(false),
        |done, total| eprint!("\rRun {done}/{total}"),
    );
    eprintln!();
    let report = match result {
        Ok(report) => report,
        Err(e) => {
            eprintln!("Campaign failed: {e}");
            return 1;
        }
    };
    let folder = std::path::Path::new(&parsed.folder);
    let text = summary(&report);
    if let Err(e) = report
        .write(folder)
        .and_then(|()| std::fs::write(folder.join("summary.txt"), &text).map_err(|e| e.to_string()))
    {
        eprintln!("Could not write the report: {e}");
        return 1;
    }
    eprint!("{text}");
    if report.errors() > 0 {
        1
    } else if report.passed() < report.runs.len() {
        2
    } else {
        0
    }
}

/// `--rerecord <in.pelagic-run> <out.pelagic-run>`: re-simulates a
/// recording on this build (same scene, checkpoint and commands) so that
/// Verify passes again. Returns the process exit code.
pub fn rerecord(args: &[String]) -> i32 {
    attach_console();
    let (Some(input), Some(output)) = (args.get(2), args.get(3)) else {
        eprintln!("Usage: Pelagic.exe --rerecord <in.pelagic-run> <out.pelagic-run>");
        return 1;
    };
    let result = std::fs::File::open(input)
        .map_err(|e| e.to_string())
        .and_then(pelagic::experiment::Recording::read)
        .and_then(|run| pelagic::experiment::rerecord(&run, &AtomicBool::new(false)))
        .and_then(|run| {
            pelagic::experiment::verify(&run, &AtomicBool::new(false))?;
            pelagic::experiment::write_atomic_with_replace(std::path::Path::new(output), &run, true)
                .map(|_| run.frames.len())
        });
    match result {
        Ok(samples) => {
            eprintln!("Re-recorded {samples} samples on this build: {output}");
            0
        }
        Err(e) => {
            eprintln!("Re-record failed: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }
    #[test]
    fn spill_and_inspect_arguments_parse_and_reject_mistakes() {
        let p = parse_spill(&args(&[
            "x",
            "--spill",
            "15.2",
            "73.2",
            "out",
            "--oil=asmb",
            "--volume=250",
            "--ago=3",
            "--wind=8,270",
            "--current=0.3,180",
            "--wind-factor=2.5",
        ]))
        .unwrap();
        assert_eq!(p.setup.product, "ASMB");
        assert_eq!((p.setup.volume_m3, p.ago), (250.0, 3.0));
        assert!((p.setup.wind_factor - 0.025).abs() < 1e-12);
        assert_eq!(p.steady, Some([8.0, 270.0, 0.3, 180.0]));
        assert!(p.project.is_none() && p.folder == "out");
        // Negative coordinates are positions, not options.
        assert!(parse_spill(&args(&["x", "--spill", "-33.9", "18.4", "o"])).is_ok());
        for bad in [
            &["x", "--spill", "15", "73"][..],
            &["x", "--spill", "15", "73", "o", "--oil=diesel"],
            &["x", "--spill", "15", "73", "o", "--volume=0"],
            &["x", "--spill", "15", "73", "o", "--current=1,2"],
            &["x", "--spill", "15", "73", "o", "--ago=99"],
        ] {
            assert!(parse_spill(&args(bad)).is_err(), "{bad:?}");
        }
        let p = parse_inspect(&args(&[
            "x",
            "--inspect",
            "demo",
            "out",
            "--gsd=0.5",
            "--pixels=4096x2160",
            "--overlap=40",
            "--vehicle=Reference vehicle",
        ]))
        .unwrap();
        assert_eq!(p.setup.gsd_mm, 0.5);
        assert_eq!(p.setup.camera.pixels, [4096.0, 2160.0]);
        assert!((p.setup.overlap - 0.4).abs() < 1e-12);
        assert_eq!(p.target, "pilings");
        assert_eq!(p.vehicle.as_deref(), Some("Reference vehicle"));
        for bad in [
            &["x", "--inspect", "demo"][..],
            &["x", "--inspect", "demo", "o", "--pixels=4k"],
            &["x", "--inspect", "demo", "o", "--gsd=0"],
            &["x", "--inspect", "demo", "o", "--sonar=yes"],
        ] {
            assert!(parse_inspect(&args(bad)).is_err(), "{bad:?}");
        }
    }
    #[test]
    fn colregs_arguments_parse_and_reject_mistakes() {
        let p = parse_colregs(&args(&[
            "x",
            "--colregs",
            "boat.json",
            "out",
            "--random=5",
            "--seed=10",
            "--safe=120",
        ]))
        .unwrap();
        assert_eq!((p.random, p.seed, p.safe), (5, 10, Some(120.0)));
        assert!(p.standard);
        let p = parse_colregs(&args(&["x", "--colregs", "b", "o", "--only=standard"])).unwrap();
        assert_eq!(p.random, 0);
        for bad in [
            &["x", "--colregs", "b"][..],
            &["x", "--colregs", "b", "o", "--only=weapons"],
            &["x", "--colregs", "b", "o", "--safe=1"],
            &["x", "--colregs", "b", "o", "--random=900"],
            &["x", "--colregs", "b", "o", "--fast"],
        ] {
            assert!(parse_colregs(&args(bad)).is_err(), "{bad:?}");
        }
    }
    #[test]
    fn survey_arguments_parse_and_reject_mistakes() {
        let p = parse_survey(&args(&[
            "Pelagic.exe",
            "--survey",
            "demo",
            "out",
            "--sensor=sss",
            "--range=50",
            "--altitude=8",
            "--order=special",
            "--overlap=20",
            "--area=10,-20,300,200",
            "--heading=contours",
            "--vehicle=Survey AUV",
        ]))
        .unwrap();
        assert_eq!(p.setup.sonar.kind, pelagic::survey::SonarKind::SideScan);
        assert!(p.setup.platform.underwater);
        assert_eq!(p.setup.sonar.range_m, 50.0);
        assert_eq!(p.setup.order().name, "Special Order");
        assert!((p.setup.overlap - 0.2).abs() < 1e-12);
        assert_eq!(p.area, Some([10.0, -20.0, 300.0, 200.0]));
        assert!(p.contours);
        assert_eq!(p.vehicle.as_deref(), Some("Survey AUV"));
        let p = parse_survey(&args(&["x", "--survey", "demo", "o", "--order=1b"])).unwrap();
        assert_eq!(p.setup.order().name, "Order 1b");
        for bad in [
            &["x", "--survey", "demo"][..],
            &["x", "--survey", "demo", "o", "--sensor=lidar"],
            &["x", "--survey", "demo", "o", "--order=3"],
            &["x", "--survey", "demo", "o", "--area=1,2,3"],
            &["x", "--survey", "demo", "o", "--overlap=95"],
            &["x", "--survey", "demo", "o", "--colour=red"],
        ] {
            assert!(parse_survey(&args(bad)).is_err(), "{bad:?}");
        }
    }
    #[test]
    fn sar_arguments_parse_and_reject_mistakes() {
        let p = parse_sar(&args(&[
            "Pelagic.exe",
            "--sar",
            "15.2",
            "73.2",
            "out",
            "--object=life-raft-db-15",
            "--ago=3",
            "--speed=12",
            "--sweep=600",
            "--pattern=ps",
            "--wind=8,270",
            "--current=0.3,180",
            "--definite",
        ]))
        .unwrap();
        assert_eq!(p.setup.object, "LIFE-RAFT-DB-15");
        assert_eq!(p.ago, 3.0);
        assert!((p.searcher.speed_mps - 12.0 * 0.514_444).abs() < 1e-9);
        assert_eq!(p.searcher.sweep_width_m, 600.0);
        assert!(p.searcher.definite_range && p.searcher.follow_drift);
        assert_eq!(p.patterns, vec![pelagic::sar::Pattern::ParallelTrack]);
        assert_eq!(p.steady, Some([8.0, 270.0, 0.3, 180.0]));
        for bad in [
            &["x", "--sar", "15", "73"][..],
            &["x", "--sar", "15", "73", "o", "--object=whale"],
            &["x", "--sar", "15", "73", "o", "--pattern=spiral"],
            &["x", "--sar", "15", "73", "o", "--current=0.3,10"],
            &["x", "--sar", "95", "73", "o"],
            &["x", "--sar", "15", "73", "o", "--ago=60"],
            &["x", "--sar", "15", "73", "o", "--colour=red"],
        ] {
            assert!(parse_sar(&args(bad)).is_err(), "{bad:?}");
        }
    }
    #[test]
    fn trials_arguments_parse_and_reject_mistakes() {
        let p = parse_trials(&args(&[
            "Pelagic.exe",
            "--trials",
            "boat.pelagic-vehicle.json",
            "out",
            "--only=turning,zigzag",
            "--rudder=20",
            "--imo",
            "--max-roll-rao=2",
        ]))
        .unwrap();
        assert_eq!(p.vehicle, "boat.pelagic-vehicle.json");
        assert!(p.setup.turning && p.setup.zigzag && !p.setup.waves && !p.setup.free_decay);
        assert_eq!(p.setup.rudder_deg, 20.0);
        assert!(p.imo);
        assert_eq!(p.criteria.max_roll_rao, Some(2.0));
        for bad in [
            &["x", "--trials", "a.json"][..],
            &["x", "--trials", "a", "b", "--only=dance"],
            &["x", "--trials", "a", "b", "--rudder=90"],
            &["x", "--trials", "a", "b", "--speed=3"],
        ] {
            assert!(parse_trials(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn campaign_arguments_parse_and_reject_mistakes() {
        let p = parse_campaign(&args(&[
            "Pelagic.exe",
            "--campaign",
            "a.pelagic",
            "out",
            "--runs=5",
            "--seed=9",
            "--max-nav-error=0.5",
            "--min-clearance=1",
        ]))
        .unwrap();
        assert_eq!(
            (p.project.as_str(), p.folder.as_str()),
            ("a.pelagic", "out")
        );
        assert_eq!((p.variation.runs, p.variation.seed_base), (5, 9));
        assert_eq!(p.criteria.max_nav_error_m, Some(0.5));
        assert_eq!(p.criteria.min_seabed_clearance_m, Some(1.0));
        assert_eq!(p.criteria.max_tilt_deg, None);
        assert_eq!(p.seconds, 30.0);
        for bad in [
            &["x", "--campaign", "a.pelagic"][..],
            &["x", "--campaign", "a", "b", "--runs=0"],
            &["x", "--campaign", "a", "b", "--runs=many"],
            &["x", "--campaign", "a", "b", "--max-speed=3"],
        ] {
            assert!(parse_campaign(&args(bad)).is_err(), "{bad:?}");
        }
    }
}
