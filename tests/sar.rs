//! Search and rescue: leeway drift against the published mean model,
//! forecast conventions, search-pattern geometry, detection theory
//! (inverse cube law POD = erf(√π·C/2)) and the end-to-end plan.
use pelagic::metocean::{Conditions, Forecast};
use pelagic::sar::{
    self, Area, DriftSetup, Drivers, Pattern, Plan, Plane, Searcher, drift, evaluate,
    expanding_square, parallel, sector,
};
use std::sync::atomic::AtomicBool;

fn no() -> AtomicBool {
    AtomicBool::new(false)
}

#[test]
fn leeway_drift_matches_the_mean_model() {
    // Person in the HELP position (PIW-3: downwind 1.60 % of U10 − 3.98 cm/s,
    // symmetric crosswind) in a steady 10 m/s northerly and a 0.2 m/s
    // current towards the east, for 10 hours.
    let drivers = Drivers::steady("2026-11-24T00:00", 24, 15.0, 73.0, 10.0, 0.0, 0.2, 90.0);
    let setup = DriftSetup {
        object: "PIW-3".into(),
        latitude: 15.0,
        longitude: 73.0,
        radius_m: 0.0,
        window_h: 0.0,
        particles: 4000,
        ..Default::default()
    };
    let d = drift(&drivers, &setup, 10.0, &no()).unwrap();
    let s = d.spread(10.0);
    let south = (1.60 * 10.0 - 3.98) * 0.01 * 36_000.0;
    let east = 0.2 * 36_000.0;
    assert!(
        (s.datum[1] + south).abs() < 150.0,
        "downwind drift {} m, expected {south} m south",
        -s.datum[1]
    );
    assert!(
        (s.datum[0] - east).abs() < 150.0,
        "current drift {} m, expected {east} m east",
        s.datum[0]
    );
    // Spread grows from the current error (0.1 m/s → 3.6 km in 10 h).
    assert!(s.major_m > 2500.0 && s.major_m < 5000.0, "{s:?}");
    assert_eq!(s.released, 4000);
    // Geographic conversion: back and forth.
    let (lat, lon) = d.plane.geo(s.datum);
    let back = d.plane.local(lat, lon);
    assert!((back[0] - s.datum[0]).abs() < 0.01 && (back[1] - s.datum[1]).abs() < 0.01);
    assert!(lat < 15.0 && lon > 73.0);
}

#[test]
fn drift_is_deterministic_per_seed_and_respects_the_incident_window() {
    let drivers = Drivers::steady("2026-11-24T00:00", 12, 0.0, 0.0, 8.0, 45.0, 0.3, 200.0);
    let setup = DriftSetup {
        particles: 500,
        incident_hour: 1.0,
        window_h: 2.0,
        ..Default::default()
    };
    let a = drift(&drivers, &setup, 6.0, &no()).unwrap();
    let b = drift(&drivers, &setup, 6.0, &no()).unwrap();
    assert_eq!(a.frames, b.frames);
    let c = drift(
        &drivers,
        &DriftSetup {
            seed: 2,
            ..setup.clone()
        },
        6.0,
        &no(),
    )
    .unwrap();
    assert_ne!(a.frames, c.frames);
    // Half way through the window about half are in the water.
    let half = a.spread(2.0).released as f64 / 500.0;
    assert!((0.4..0.6).contains(&half), "{half}");
    assert_eq!(a.spread(3.01).released, 500);
    // Outside the data, or a search before the incident, is refused.
    assert!(drift(&drivers, &setup, 30.0, &no()).is_err());
    assert!(drift(&drivers, &setup, 0.5, &no()).is_err());
}

#[test]
fn forecasts_convert_with_wind_from_and_current_towards() {
    let mut f = Forecast {
        latitude: 10.0,
        longitude: 70.0,
        source: "test".into(),
        hours: vec![],
    };
    for h in 0..3 {
        f.hours.push(Conditions {
            time: format!("2026-11-24T{h:02}:00"),
            wind_speed: Some(5.0),
            wind_direction: Some(0.0),
            current_speed: Some(0.5),
            current_direction: Some(90.0),
            ..Default::default()
        });
    }
    // A gap is filled from the previous hour.
    f.hours[1].wind_speed = None;
    let mut land = f.clone();
    land.longitude = 70.5;
    for h in &mut land.hours {
        h.current_speed = None;
        h.current_direction = None;
    }
    let d = Drivers::from_forecasts(&[f, land]).unwrap();
    assert_eq!(d.stations.len(), 1);
    assert_eq!(d.notes.len(), 1);
    let s = &d.stations[0];
    // Wind from the north blows towards the south.
    assert!(s.wind[0][0].abs() < 1e-9 && (s.wind[0][1] + 5.0).abs() < 1e-9);
    assert_eq!(s.wind[1], s.wind[0]);
    assert!((s.current[0][0] - 0.5).abs() < 1e-9 && s.current[0][1].abs() < 1e-9);
    assert_eq!(d.hour_of("2026-11-24T02:30"), Some(2.5));
    assert_eq!(d.time_of(1.5), "2026-11-24T01:30");
    assert!((sar::bearing([1.0, 0.0]) - 90.0).abs() < 1e-9);
}

#[test]
fn search_patterns_have_the_standard_geometry() {
    let p = expanding_square([0.0, 0.0], 0.0, 100.0, 1000.0);
    let expected = [
        [0.0, 0.0],
        [0.0, 100.0],
        [100.0, 100.0],
        [100.0, -100.0],
        [-100.0, -100.0],
        [-100.0, 200.0],
        [0.0, 200.0],
    ];
    assert_eq!(p.len(), expected.len());
    for (a, b) in p.iter().zip(expected) {
        assert!(
            (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6,
            "{p:?}"
        );
    }
    // Sector: nine legs of R, back over the datum every third leg.
    let r = 500.0;
    let v = sector([10.0, 20.0], 30.0, r, 9.0 * r);
    assert_eq!(v.len(), 10);
    for i in [0, 3, 6, 9] {
        assert!(
            (v[i][0] - 10.0).abs() < 1e-6 && (v[i][1] - 20.0).abs() < 1e-6,
            "{v:?}"
        );
    }
    // Parallel track and creeping line stay in the area and fit the budget.
    let area = Area {
        centre: [100.0, -50.0],
        axis_deg: 60.0,
        length_m: 4000.0,
        width_m: 1000.0,
        round: false,
    };
    for creeping in [false, true] {
        let w = parallel(&area, 200.0, 1e9, creeping);
        let grown = Area {
            length_m: area.length_m + 1.0,
            width_m: area.width_m + 1.0,
            ..area
        };
        assert!(w.iter().all(|p| grown.contains(*p)), "{w:?}");
        let plan = Plan {
            pattern: Pattern::ParallelTrack,
            spacing_m: 200.0,
            area,
            drift_velocity: [0.0, 0.0],
            start_hour: 0.0,
            waypoints: w,
        };
        let full = plan.track_m();
        let expected = if creeping {
            20.0 * 1000.0 + 19.0 * 200.0
        } else {
            5.0 * 4000.0 + 4.0 * 200.0
        };
        assert!((full - expected).abs() < 1e-6, "{full} vs {expected}");
        let cut = parallel(&area, 200.0, 7_000.0, creeping);
        let plan = Plan {
            waypoints: cut,
            ..plan
        };
        assert!((plan.track_m() - 7_000.0).abs() < 1e-6);
    }
    assert_eq!(Pattern::parse("ps"), Some(Pattern::ParallelTrack));
    assert_eq!(
        Pattern::parse("expanding-square"),
        Some(Pattern::ExpandingSquare)
    );
}

/// Static particles uniform in the inner part of a large parallel sweep.
fn sweep_pod(width: f64, spacing: f64, definite: bool) -> f64 {
    let plane = Plane {
        latitude: 0.0,
        longitude: 0.0,
    };
    let mut state = 7u64;
    let positions: Vec<[f64; 2]> = (0..4000)
        .map(|_| {
            [
                (pelagic::life::random(&mut state) - 0.5) * 2000.0,
                (pelagic::life::random(&mut state) - 0.5) * 4000.0,
            ]
        })
        .collect();
    let d = sar::Drift::stationary(&sar::OBJECTS[0], plane, positions, 8.0);
    let area = Area {
        centre: [0.0, 0.0],
        axis_deg: 0.0,
        length_m: 6000.0,
        width_m: 3000.0,
        round: false,
    };
    let searcher = Searcher {
        speed_mps: 20.0,
        sweep_width_m: width,
        definite_range: definite,
        start_hour: 0.0,
        endurance_h: 7.5,
        follow_drift: false,
        ..Default::default()
    };
    let plan = Plan {
        pattern: Pattern::ParallelTrack,
        spacing_m: spacing,
        area,
        drift_velocity: [0.0, 0.0],
        start_hour: 0.0,
        waypoints: parallel(&area, spacing, searcher.budget_m(), false),
    };
    let o = evaluate(&d, &searcher, &plan);
    assert!((o.poc - 1.0).abs() < 1e-9);
    assert!(o.pos_curve.windows(2).all(|w| w[1][1] >= w[0][1]));
    o.pod
}

#[test]
fn inverse_cube_parallel_sweep_matches_koopman() {
    // POD = erf(√π·C/2): C = 1 → 0.7900, C = 2 → 0.9876.
    let one = sweep_pod(100.0, 100.0, false);
    assert!((one - 0.7900).abs() < 0.02, "C = 1: POD {one}");
    let two = sweep_pod(100.0, 50.0, false);
    assert!((two - 0.9876).abs() < 0.01, "C = 2: POD {two}");
    // Definite range: complete at S = W, half at S = 2W.
    let full = sweep_pod(100.0, 100.0, true);
    assert!(full > 0.99, "definite S = W: {full}");
    let half = sweep_pod(100.0, 200.0, true);
    assert!((half - 0.5).abs() < 0.03, "definite S = 2W: {half}");
}

#[test]
fn a_full_plan_ranks_patterns_and_writes_its_files() {
    let drivers = Drivers::steady("2026-11-24T00:00", 24, 15.2, 73.2, 9.0, 300.0, 0.25, 150.0);
    let setup = DriftSetup {
        latitude: 15.2,
        longitude: 73.2,
        radius_m: 500.0,
        incident_hour: 1.0,
        window_h: 0.5,
        particles: 1500,
        ..Default::default()
    };
    let searcher = Searcher {
        start_hour: 3.0,
        endurance_h: 3.0,
        sweep_width_m: 300.0,
        ..Default::default()
    };
    let report = sar::run(&drivers, &setup, &searcher, &[], 4, &no(), |_, _| {}).unwrap();
    assert!(report.options.len() >= 15);
    assert!(report.options.windows(2).all(|w| w[0].pos >= w[1].pos));
    for o in &report.options {
        assert!((0.0..=1.0).contains(&o.pos) && (0.0..=1.0).contains(&o.poc));
        assert!(o.track_m <= searcher.budget_m() + 1.0);
        assert!(o.duration_h <= searcher.endurance_h + 1e-9);
    }
    let best = &report.options[0];
    assert!(best.pos > 0.3, "best POS {}", best.pos);
    let patterns: std::collections::HashSet<_> =
        report.options.iter().map(|o| o.plan.pattern).collect();
    assert_eq!(patterns.len(), 4);
    let folder = std::env::temp_dir().join(format!("pelagic-sar-test-{}", std::process::id()));
    report.write(&folder, 0).unwrap();
    for name in [
        "summary.txt",
        "drift.csv",
        "particles.csv",
        "plans.json",
        "plan.gpx",
        "plan.waypoints",
    ] {
        assert!(folder.join(name).exists(), "{name}");
    }
    let wpl = std::fs::read_to_string(folder.join("plan.waypoints")).unwrap();
    assert!(wpl.starts_with("QGC WPL 110\n0\t1\t0\t16\t"));
    let gpx = std::fs::read_to_string(folder.join("plan.gpx")).unwrap();
    assert!(gpx.contains("<rtept") && gpx.contains("LKP"));
    let _ = std::fs::remove_dir_all(&folder);
    // A search that starts before the incident is refused.
    let early = Searcher {
        start_hour: 0.5,
        ..searcher
    };
    assert!(sar::run(&drivers, &setup, &early, &[], 2, &no(), |_, _| {}).is_err());
}

#[test]
fn camera_sweep_width_is_limited_by_pixels_and_horizon() {
    // 0.5 m object, 60° over 1920 px, 10 px needed: R ≈ 91.7 m, W ≈ 183 m.
    let w = sar::camera_sweep_width(0.5, 10.0, 60.0, 1920.0, 10.0);
    assert!((w - 183.3).abs() < 1.0, "{w}");
    // A large object from a low eye is limited by the horizon.
    let w = sar::camera_sweep_width(15.0, 1.0, 10.0, 4000.0, 2.0);
    assert!((w - 2.0 * sar::horizon_m(1.0, 4.5)).abs() < 1e-6);
    assert!(sar::find_object("piw-1").is_some());
    assert_eq!(sar::OBJECTS.len(), 21);
}
