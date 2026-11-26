//! Seabed survey: IHO S-44 limits, swath and uncertainty geometry, adaptive
//! line spacing, side-scan blind zones, coverage measurement and a physics
//! run with the example workboat.
use pelagic::sar::Area;
use pelagic::scene::Scene;
use pelagic::survey::{
    self, ORDERS, Platform, Setup, Sonar, SonarKind, contour_heading, ideal, plan,
};
use pelagic::terrain::HeightGrid;
use std::sync::atomic::AtomicBool;

/// A 1.2 km square seabed: depth(x) = d0 + slope·(x + 600).
fn bed(d0: f64, slope: f64) -> HeightGrid {
    let (n, spacing) = (121, 10.0);
    let mut z = vec![];
    for _j in 0..n {
        for i in 0..n {
            let x = i as f64 * spacing - 600.0;
            z.push(-(d0 + slope * (x + 600.0)));
        }
    }
    HeightGrid::new(n, n, [-600.0, -600.0], spacing, z).unwrap()
}

fn area(length: f64, width: f64, axis: f64) -> Area {
    Area {
        centre: [0.0, 0.0],
        axis_deg: axis,
        length_m: length,
        width_m: width,
        round: false,
    }
}

#[test]
fn s44_limits_and_sonar_uncertainty() {
    let o1a = &ORDERS[2];
    assert_eq!(o1a.name, "Order 1a");
    // √(0.5² + (0.013·40)²) = 0.7214 m.
    assert!((o1a.tvu(40.0) - 0.7214).abs() < 1e-4);
    assert_eq!(o1a.feature(30.0), Some(2.0));
    assert!((o1a.feature(60.0).unwrap() - 6.0).abs() < 1e-9);
    assert!((ORDERS[3].tvu(20.0) - 0.2915).abs() < 1e-4);
    assert_eq!(ORDERS[4].passes(), 2);
    assert_eq!(ORDERS[0].feature(10.0), None);
    let s = Sonar::multibeam();
    // The TVU-limited angle gives exactly the allowed uncertainty.
    for h in [5.0, 30.0, 120.0] {
        let allowed = ORDERS[2].tvu(h);
        let theta = s.tvu_angle(h, allowed).unwrap();
        assert!((s.tvu(h, theta) - allowed).abs() < 1e-9, "h {h}");
        assert!(s.tvu(h, 0.0) < allowed);
    }
    // Too deep for the fixed and range parts alone.
    let poor = Sonar {
        sigma_fixed_m: 0.2,
        ..Sonar::multibeam()
    };
    assert!(poor.tvu_angle(30.0, ORDERS[4].tvu(30.0)).is_none());
}

#[test]
fn multibeam_lines_cover_a_flat_seabed_once_or_twice() {
    let seabed = bed(30.0, 0.0);
    let setup = Setup {
        area: area(500.0, 400.0, 90.0),
        ..Default::default()
    };
    let lines = plan(&seabed, &setup).unwrap();
    // Swath-limited: half width 29.5·tan 60° = 51.1 m, spacing 2·51.1·0.9.
    let spacing = (lines[1].offset_m - lines[0].offset_m).abs();
    assert!(
        (spacing - 2.0 * 29.5 * 3f64.sqrt() * 0.9).abs() < 1.0,
        "{spacing}"
    );
    assert!((4..=6).contains(&lines.len()), "{}", lines.len());
    let run = ideal(&seabed, &setup, &lines);
    assert!(run.stats.covered_percent > 99.5, "{:?}", run.stats);
    assert!(run.stats.meets_order && run.stats.gaps == 0);
    let tvu = run.stats.tvu_mean_m.unwrap();
    assert!(tvu > 0.05 && tvu < ORDERS[2].tvu(30.0), "{tvu}");
    // Lines alternate direction and fill the area length.
    let d0 = lines[0].end[0] - lines[0].start[0];
    let d1 = lines[1].end[0] - lines[1].start[0];
    assert!(d0 * d1 < 0.0 && (d0.abs() - 500.0).abs() < 1e-6);
    // Exclusive Order: 200 % coverage needs about twice the lines.
    let exclusive = Setup { order: 4, ..setup };
    let more = plan(&seabed, &exclusive).unwrap();
    assert!(
        more.len() >= 2 * lines.len() - 2,
        "{} vs {}",
        more.len(),
        lines.len()
    );
    let run = ideal(&seabed, &exclusive, &more);
    assert_eq!(run.stats.required_passes, 2);
    assert!(run.stats.multiple_percent > 97.0, "{:?}", run.stats);
}

#[test]
fn spacing_adapts_to_depth_and_contours_set_the_heading() {
    // 10 m deep in the west to 70 m in the east; lines run north–south.
    let seabed = bed(10.0, 0.05);
    let a = area(600.0, 800.0, 0.0);
    let heading = contour_heading(&seabed, &a);
    assert!(heading < 2.0 || heading > 178.0, "{heading}");
    let setup = Setup {
        area: a,
        order: 1,
        ..Default::default()
    };
    let lines = plan(&seabed, &setup).unwrap();
    let gaps: Vec<f64> = lines
        .windows(2)
        .map(|w| (w[1].offset_m - w[0].offset_m).abs())
        .collect();
    // Area axis north: +across is east (deeper), so spacing widens (the
    // last line sits on the boundary).
    let widest = gaps.iter().cloned().fold(0.0, f64::max);
    assert!(gaps[0] * 2.0 < widest, "{gaps:?}");
    assert!(
        gaps.windows(2).take(gaps.len() - 2).all(|w| w[1] > w[0]),
        "{gaps:?}"
    );
    let run = ideal(&seabed, &setup, &lines);
    assert!(run.stats.covered_percent > 99.0, "{:?}", run.stats);
}

#[test]
fn side_scan_blind_zone_is_covered_by_the_neighbours() {
    let seabed = bed(25.0, 0.0);
    let setup = Setup {
        area: area(400.0, 500.0, 30.0),
        sonar: Sonar::side_scan(),
        platform: Platform {
            underwater: true,
            altitude_m: 10.0,
            draft_m: 0.0,
        },
        ..Default::default()
    };
    assert_eq!(setup.sonar.kind, SonarKind::SideScan);
    let lines = plan(&seabed, &setup).unwrap();
    // Reach √(75² − 10²) = 74.3 m, blind zone 10 m: spacing (74.3 − 10)·0.9.
    let spacing = (lines[1].offset_m - lines[0].offset_m).abs();
    assert!(
        (spacing - ((75f64.powi(2) - 100.0).sqrt() - 10.0) * 0.9).abs() < 1.0,
        "{spacing}"
    );
    let run = ideal(&seabed, &setup, &lines);
    assert!(run.stats.covered_percent > 99.0, "{:?}", run.stats);
    assert!(run.stats.tvu_mean_m.is_none());
    // One line alone leaves its blind zone uncovered.
    let one = ideal(&seabed, &setup, &lines[1..2]);
    let g = &one.coverage;
    let row_at = |offset: f64| ((offset + 250.0) / g.cell_v) as usize;
    let k = row_at(lines[1].offset_m) * g.cols + g.cols / 2;
    assert_eq!(g.passes[k], 0, "nadir of a side-scan line is blind");
}

#[test]
fn a_workboat_runs_the_lines_with_its_physics() {
    let text =
        std::fs::read_to_string("examples/vehicles/workboat-usv.pelagic-vehicle.json").unwrap();
    let boat = pelagic::definition::VehicleDefinition::parse(&text)
        .unwrap()
        .to_entity(1, pelagic::math::Vec3::ZERO, 0.0);
    let scene = Scene {
        terrain: 0,
        bottom_depth: 25.0,
        structure: false,
        current: [0.0, 0.2, 0.0],
        wave_height: 0.3,
        ..Scene::default()
    };
    let setup = Setup {
        area: area(160.0, 120.0, 90.0),
        speed_mps: 2.0,
        ..Default::default()
    };
    let report = survey::run(&scene, &setup, Some(&boat), &AtomicBool::new(false), |_| {}).unwrap();
    let sim = report.simulated.as_ref().unwrap();
    assert_eq!(sim.lines_done, report.lines.len(), "{:?}", sim.notes);
    assert!(sim.cross_track_rms_m < 3.0, "xte {}", sim.cross_track_rms_m);
    assert!(sim.stats.covered_percent > 85.0, "{:?}", sim.stats);
    assert!(report.ideal.stats.covered_percent >= sim.stats.covered_percent - 1.0);
    assert!(sim.hours > 0.0 && !sim.track.is_empty());
    let folder = std::env::temp_dir().join(format!("pelagic-survey-test-{}", std::process::id()));
    report.write(&folder).unwrap();
    for name in [
        "summary.txt",
        "survey.json",
        "lines.csv",
        "coverage.asc",
        "tvu.asc",
        "depth.asc",
    ] {
        assert!(folder.join(name).exists(), "{name}");
    }
    // The default scene has a geographic origin: lines export as GPX/QGC.
    assert!(folder.join("lines.gpx").exists() && folder.join("survey.waypoints").exists());
    let asc = std::fs::read_to_string(folder.join("coverage.asc")).unwrap();
    assert!(asc.starts_with("ncols "));
    let _ = std::fs::remove_dir_all(&folder);
}
