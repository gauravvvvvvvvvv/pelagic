use pelagic::{
    campaign::{self, Criteria, Metrics, Variation},
    scene::Scene,
};
use std::sync::atomic::AtomicBool;

fn quick(runs: usize, threads: usize, criteria: &Criteria) -> campaign::Report {
    let variation = Variation {
        runs,
        seed_base: 100,
        wave_height_jitter: 0.3,
        current_jitter: 0.2,
    };
    campaign::run(
        &Scene::robotics_demo(),
        1.0,
        &variation,
        criteria,
        threads,
        &AtomicBool::new(false),
        |_, _| {},
    )
    .unwrap()
}

#[test]
fn campaigns_are_deterministic_and_independent_of_thread_count() {
    let criteria = Criteria::default();
    let one = quick(4, 1, &criteria);
    let many = quick(4, 4, &criteria);
    assert_eq!(one, many);
    assert_eq!(one.runs.len(), 4);
    assert!(one.runs.iter().enumerate().all(|(i, r)| r.index == i));
    assert!(one.runs.iter().all(|r| r.error.is_none() && r.passed));
    assert!(one.runs.iter().all(|r| r.metrics.samples > 5));
    // Seeds change sensor noise, so navigation errors differ between runs.
    let errors: Vec<f64> = one.runs.iter().map(|r| r.metrics.nav_error_max_m).collect();
    assert!(errors.iter().any(|e| *e != errors[0]), "{errors:?}");
    assert!(errors.iter().all(|e| *e > 0.0 && e.is_finite()));
}

#[test]
fn any_run_is_reproducible_from_its_index() {
    let criteria = Criteria::default();
    let report = quick(3, 2, &criteria);
    let again = campaign::run_one(
        &Scene::robotics_demo(),
        1.0,
        &report.variation,
        &criteria,
        2,
        &AtomicBool::new(false),
    );
    assert_eq!(again, report.runs[2]);
    let scene = report.variation.scene(&Scene::robotics_demo(), 2);
    assert_eq!(scene.seed, 102);
    assert_eq!(scene.wave_height, report.runs[2].wave_height_m);
}

#[test]
fn jitter_stays_within_its_bounds() {
    let base = Scene::robotics_demo();
    let variation = Variation {
        runs: 200,
        seed_base: 7,
        wave_height_jitter: 0.25,
        current_jitter: 0.5,
    };
    let mut spread = (f64::INFINITY, f64::NEG_INFINITY);
    for i in 0..variation.runs {
        let s = variation.scene(&base, i);
        let ratio = s.wave_height / base.wave_height;
        assert!((0.75..=1.25).contains(&ratio), "{ratio}");
        spread = (spread.0.min(ratio), spread.1.max(ratio));
        for (c, b) in s.current.iter().zip(base.current) {
            if b != 0.0 {
                assert!((0.5..=1.5).contains(&(c / b)));
            }
        }
    }
    assert!(
        spread.1 - spread.0 > 0.3,
        "jitter should actually vary: {spread:?}"
    );
    assert!(
        Variation {
            runs: 0,
            ..Default::default()
        }
        .validate()
        .is_err()
    );
}

#[test]
fn criteria_fail_exactly_the_limits_that_are_exceeded() {
    let m = Metrics {
        samples: 10,
        nav_error_max_m: 0.5,
        depth_excursion_max_m: 0.2,
        seabed_clearance_min_m: 1.0,
        vehicle_separation_min_m: f64::INFINITY,
        tilt_max_deg: 4.0,
        sonar_coverage_mean: 0.9,
        ..Default::default()
    };
    assert!(Criteria::default().judge(&m).is_empty());
    let strict = Criteria {
        max_nav_error_m: Some(0.4),
        max_depth_excursion_m: Some(0.5),
        min_seabed_clearance_m: Some(2.0),
        min_vehicle_separation_m: Some(1.0),
        max_tilt_deg: Some(10.0),
        min_sonar_coverage: Some(0.95),
    };
    let failures = strict.judge(&m);
    assert_eq!(failures.len(), 3, "{failures:?}");
    assert!(failures[0].starts_with("nav error"));
    assert!(failures.iter().any(|f| f.starts_with("seabed clearance")));
    assert!(failures.iter().any(|f| f.starts_with("sonar coverage")));
    // A metric with no data fails an enabled check instead of passing it.
    let empty = Metrics {
        nav_error_max_m: f64::NAN,
        ..m
    };
    assert_eq!(
        Criteria {
            max_nav_error_m: Some(10.0),
            ..Default::default()
        }
        .judge(&empty)
        .len(),
        1
    );
}

#[test]
fn a_tight_limit_fails_runs_and_reports_export() {
    let criteria = Criteria {
        max_nav_error_m: Some(1e-6),
        ..Default::default()
    };
    let report = quick(2, 2, &criteria);
    assert_eq!(report.passed(), 0);
    assert!(report.runs.iter().all(|r| r.failures.len() == 1));
    let csv = report.csv();
    assert_eq!(csv.lines().count(), 3);
    assert!(csv.starts_with("index,seed,"));
    let worst = report.worst();
    assert!(worst.nav_error_max_m >= report.runs[0].metrics.nav_error_max_m);
    let folder = std::env::temp_dir().join(format!("pelagic-campaign-{}", std::process::id()));
    report.write(&folder).unwrap();
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(folder.join("campaign.json")).unwrap()).unwrap();
    assert_eq!(json["format"], "pelagic-campaign");
    assert_eq!(json["runs"].as_array().unwrap().len(), 2);
    assert!(folder.join("campaign.csv").exists());
    std::fs::remove_dir_all(&folder).unwrap();
}
