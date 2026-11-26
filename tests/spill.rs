//! Oil spill: Fingas evaporation, Fay spreading, drift = current + wind
//! factor, diffusion, mass conservation and stranding on a coastline.
use pelagic::sar::Drivers;
use pelagic::spill::{
    self, Land, PRODUCTS, SpillSetup, distillation_coefficients, evaporated, fay_radius,
};
use pelagic::terrain::HeightGrid;
use std::sync::atomic::AtomicBool;

fn no() -> AtomicBool {
    AtomicBool::new(false)
}

#[test]
fn evaporation_and_spreading_follow_the_published_forms() {
    // ANS at 15 °C after one day: (2.86 + 0.045·15)·ln(1440) = 25.7 %.
    let ans = PRODUCTS[0];
    let e = evaporated(ans.a, ans.b, 15.0, 1440.0);
    assert!((e - 0.2571).abs() < 0.001, "{e}");
    // Warmer water evaporates more; nothing in the first minute.
    assert!(evaporated(ans.a, ans.b, 30.0, 1440.0) > e);
    assert_eq!(evaporated(ans.a, ans.b, 15.0, 0.5), 0.0);
    // Generic distillation form: 20 % distilled at 15 °C → 3.3·ln t.
    let (a, b) = distillation_coefficients(20.0);
    assert!((a + b * 15.0 - 3.3).abs() < 1e-9);
    // Fay radius grows as V^(1/3) t^(1/4).
    let r1 = fay_radius(100.0, 870.0, 3600.0);
    let r2 = fay_radius(800.0, 870.0, 3600.0);
    assert!((r2 / r1 - 2.0).abs() < 1e-9);
    // 100 m³ after an hour: about 176 m.
    assert!((r1 - 176.0).abs() < 5.0, "{r1}");
}

#[test]
fn oil_drifts_with_the_current_and_three_percent_of_the_wind() {
    // 10 m/s from the west and 0.2 m/s towards the north, 12 hours.
    let drivers = Drivers::steady("2026-11-24T00:00", 24, 10.0, 70.0, 10.0, 270.0, 0.2, 0.0);
    let setup = SpillSetup {
        latitude: 10.0,
        longitude: 70.0,
        duration_h: 12.0,
        particles: 2000,
        ..Default::default()
    };
    let s = spill::run(&drivers, &setup, None, &no()).unwrap();
    let b = s.balance(12.0);
    let east = 0.03 * 10.0 * 12.0 * 3600.0;
    let north = 0.2 * 12.0 * 3600.0;
    assert!((b.centre[0] - east).abs() < 150.0, "{:?}", b.centre);
    assert!((b.centre[1] - north).abs() < 150.0, "{:?}", b.centre);
    // Mass is conserved and evaporation matches the parcel ages.
    assert!((b.released_m3 - 100.0).abs() < 1e-6);
    assert!((b.surface_m3 + b.evaporated_m3 + b.stranded_m3 - 100.0).abs() < 1e-6);
    let expected = evaporated(PRODUCTS[0].a, PRODUCTS[0].b, 20.0, 720.0) * 100.0;
    assert!(
        (b.evaporated_m3 - expected).abs() < 0.5,
        "{}",
        b.evaporated_m3
    );
    assert!(b.area_km2 > 0.5 && b.thickness_um > 0.0);
    // Deterministic per seed.
    let again = spill::run(&drivers, &setup, None, &no()).unwrap();
    assert_eq!(s.frames, again.frames);
    // A continuous release is only partly out half way through.
    let slow = SpillSetup {
        release_h: 6.0,
        ..setup.clone()
    };
    let c = spill::run(&drivers, &slow, None, &no()).unwrap();
    let half = c.balance(3.0).released_m3;
    assert!((half - 50.0).abs() < 3.0, "{half}");
    let folder = std::env::temp_dir().join(format!("pelagic-spill-{}", std::process::id()));
    s.write(&folder).unwrap();
    for name in [
        "summary.txt",
        "mass_balance.csv",
        "particles.csv",
        "slick.geojson",
    ] {
        assert!(folder.join(name).exists(), "{name}");
    }
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn oil_strands_on_the_coast_and_stops_there() {
    // Shore 3 km east of the release; wind blowing towards it.
    let (n, spacing) = (101, 100.0);
    let z: Vec<f64> = (0..n * n)
        .map(|k| {
            let x = (k % n) as f64 * spacing - 5000.0;
            if x > 3000.0 { 5.0 } else { -20.0 }
        })
        .collect();
    let land = Land {
        grid: HeightGrid::new(n, n, [-5000.0, -5000.0], spacing, z).unwrap(),
        offset: [0.0, 0.0],
    };
    let drivers = Drivers::steady("2026-11-24T00:00", 30, 10.0, 70.0, 12.0, 270.0, 0.1, 90.0);
    let setup = SpillSetup {
        latitude: 10.0,
        longitude: 70.0,
        duration_h: 24.0,
        particles: 1500,
        ..Default::default()
    };
    let s = spill::run(&drivers, &setup, Some(&land), &no()).unwrap();
    let (h, p) = s.first_landfall().expect("oil reaches the shore");
    // 0.46 m/s eastwards: about 1.8 h to cover 3 km.
    assert!(h > 1.0 && h < 3.0, "{h}");
    assert!((p[0] - 3000.0).abs() < 120.0, "{p:?}");
    let end = s.balance(24.0);
    assert!(end.stranded_m3 > 50.0, "{end:?}");
    assert!((end.surface_m3 + end.evaporated_m3 + end.stranded_m3 - 100.0).abs() < 1e-6);
    // No parcel ends up on land beyond the shore.
    for i in 0..s.parcels() {
        // The shoreline (elevation 0) lies at x ≈ 3080 m between grid nodes.
        assert!(s.position(i, 24.0)[0] <= 3081.0);
    }
    // A release on land is refused.
    let on_land = Land {
        offset: [4000.0, 0.0],
        ..land
    };
    assert!(spill::run(&drivers, &setup, Some(&on_land), &no()).is_err());
}
