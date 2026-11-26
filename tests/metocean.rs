//! Forecast parsing, direction conventions, applying conditions to scenes,
//! and seakeeping/operability from a computed response table.
use pelagic::{
    definition::VehicleDefinition,
    math::Vec3,
    metocean::{self, Conditions, Forecast, WaveField},
    scene::Scene,
    seakeeping::{self, Criteria},
    trials::Environment,
};
use std::sync::atomic::AtomicBool;

const MARINE: &str = r#"{"latitude":15.25,"longitude":73.0,"hourly":{
 "time":["2026-11-23T00:00","2026-11-23T01:00","2026-11-23T02:00"],
 "wave_height":[1.6,1.7,null],"wave_direction":[250,252,255],"wave_period":[6.2,6.4,6.5],
 "wind_wave_height":[0.9,1.0,1.1],"wind_wave_direction":[270,270,272],"wind_wave_period":[4.0,4.1,4.2],
 "wind_wave_peak_period":[4.8,4.9,5.0],
 "swell_wave_height":[1.3,1.35,1.4],"swell_wave_direction":[225,225,226],"swell_wave_period":[10.0,10.1,10.2],
 "swell_wave_peak_period":[11.5,11.6,11.7],
 "ocean_current_velocity":[1.8,1.9,2.0],"ocean_current_direction":[90,95,100],
 "sea_surface_temperature":[28.4,28.4,28.3]}}"#;
const WEATHER: &str = r#"{"hourly":{"time":["2026-11-23T00:00","2026-11-23T01:00"],
 "wind_speed_10m":[7.5,8.0],"wind_direction_10m":[270,275]}}"#;

fn forecast() -> Forecast {
    let marine: serde_json::Value = serde_json::from_str(MARINE).unwrap();
    let weather: serde_json::Value = serde_json::from_str(WEATHER).unwrap();
    Forecast::parse_open_meteo(&marine, Some(&weather), "Open-Meteo (test)").unwrap()
}

#[test]
fn open_meteo_forecasts_parse_with_units_and_gaps() {
    let f = forecast();
    assert_eq!(f.hours.len(), 3);
    let h = &f.hours[0];
    assert_eq!(h.wave_height, Some(1.6));
    assert_eq!(f.hours[2].wave_height, None, "null stays missing");
    // Current arrives in km/h.
    assert!((h.current_speed.unwrap() - 0.5).abs() < 1e-12);
    assert_eq!((h.wind_speed, h.wind_direction), (Some(7.5), Some(270.0)));
    assert_eq!(f.hours[2].wind_speed, None, "no weather for that hour");
    assert_eq!(f.index_at("2026-11-23T01:30"), 1);
    // Multi-location field: one object per requested point, in order.
    let list = format!("[{MARINE},{MARINE}]");
    let v: serde_json::Value = serde_json::from_str(&list).unwrap();
    let field = WaveField::parse_open_meteo(&v, &[(10.0, 60.0), (20.0, 70.0)]).unwrap();
    assert_eq!(field.times.len(), 3);
    assert_eq!(field.points[1].latitude, 20.0);
    assert_eq!(field.points[0].height[2], None);
    let err: serde_json::Value =
        serde_json::from_str(r#"{"error":true,"reason":"Too many"}"#).unwrap();
    assert!(
        Forecast::parse_open_meteo(&err, None, "x")
            .unwrap_err()
            .contains("Too many")
    );
    assert!(WaveField::parse_open_meteo(&v, &[(0.0, 0.0)]).is_err());
}

#[test]
fn directions_convert_from_compass_to_scene_axes() {
    // Travelling towards east = +X (0°), north = +Y (90°), south = −90°.
    assert!((metocean::travel_angle(90.0) - 0.0).abs() < 1e-12);
    assert!((metocean::travel_angle(0.0) - 90.0).abs() < 1e-12);
    assert!((metocean::travel_angle(180.0) + 90.0).abs() < 1e-12);
    assert_eq!(metocean::utc_hour(0), "1970-01-01T00:00");
    assert_eq!(metocean::utc_hour(951_782_400), "2000-02-29T00:00");
    assert_eq!(metocean::utc_hour(1_700_000_000), "2023-11-14T22:00");
}

#[test]
fn forecast_conditions_drive_a_valid_scene_with_wind_sea_and_swell() {
    let f = forecast();
    let mut scene = Scene {
        bottom_depth: 60.0,
        ..Default::default()
    };
    let notes = metocean::apply(&mut scene, &f.hours[0], "Open-Meteo (test)");
    assert!(notes.is_empty(), "{notes:?}");
    scene.validate().unwrap();
    assert_eq!(scene.wave_height, 0.9);
    assert_eq!(scene.wave_period, 4.8);
    // Wind sea from the west travels east: +X.
    assert!(scene.wave_direction.abs() < 1e-9);
    assert_eq!((scene.swell_height, scene.swell_period), (1.3, 11.5));
    // Current towards east at 0.5 m/s.
    assert!((scene.current[0] - 0.5).abs() < 1e-9 && scene.current[1].abs() < 1e-9);
    assert_eq!(scene.temperature, 28.4);
    assert_eq!(scene.wind_speed, 7.5);
    assert!(scene.sea_source.contains("2026-11-23T00:00"));
    // The combined sea has the total significant height.
    let sea = pelagic::scene::sea_state(&scene);
    let total = (0.9f64).hypot(1.3);
    assert!((sea.significant_height() - total).abs() < 1e-9);
    assert_eq!(sea.components().len(), 48);
    assert_eq!(sea.wind_speed, 7.5);
    // A storm in shallow water is clamped to the depth-limited envelope.
    let mut shallow = Scene {
        bottom_depth: 8.0,
        vehicle_depth: 3.0,
        ..Default::default()
    };
    let storm = Conditions {
        time: "2026-11-23T00:00".into(),
        wave_height: Some(6.0),
        wave_period: Some(9.0),
        wave_direction: Some(200.0),
        ..Default::default()
    };
    let notes = metocean::apply(&mut shallow, &storm, "test");
    assert!(!notes.is_empty() && shallow.wave_height <= 2.0);
    shallow.validate().unwrap();
    // Scenes saved before swell existed still load, without swell.
    let mut value = Scene::default().value();
    for key in [
        "swell_height",
        "swell_period",
        "swell_direction",
        "wind_speed",
        "wind_direction",
        "sea_source",
    ] {
        value["environment"].as_object_mut().unwrap().remove(key);
    }
    let old = Scene::from_value(&value).unwrap();
    assert_eq!(old.swell_height, 0.0);
    assert_eq!(Scene::from_value(&scene.value()).unwrap().swell_height, 1.3);
}

fn workboat() -> pelagic::entities::Entity {
    VehicleDefinition::parse(
        &std::fs::read_to_string("examples/vehicles/workboat-usv.pelagic-vehicle.json").unwrap(),
    )
    .unwrap()
    .to_entity(1, Vec3::ZERO, 0.0)
}

#[test]
fn seakeeping_tables_follow_physics_and_give_an_operability_outlook() {
    let setup = seakeeping::Setup {
        periods: vec![5.0, 12.0],
        headings: vec![90.0, 180.0],
        speed_fraction: 0.0,
        seconds: 90.0,
        ..seakeeping::Setup::default()
    };
    let run = |threads| {
        seakeeping::run(
            &workboat(),
            &Environment::default(),
            &setup,
            threads,
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap()
    };
    let report = run(4);
    assert_eq!(report.mode, "surface");
    assert_eq!(report.cells.len(), 4);
    let at = |t: f64, h: f64| {
        report
            .cells
            .iter()
            .find(|c| c.period_s == t && c.heading_deg == h)
            .unwrap()
    };
    // Beam seas roll the boat more than head seas; head seas pitch it more.
    assert!(at(5.0, 90.0).roll_deg > 3.0 * at(5.0, 180.0).roll_deg);
    assert!(at(5.0, 180.0).pitch_deg > at(5.0, 90.0).pitch_deg);
    // In long waves a small boat follows the surface: heave RMS ≈ Hs/4.
    let heave = at(12.0, 180.0).heave_m;
    assert!((0.2..0.3).contains(&heave), "{heave}");
    // Limiting heights are finite and RMS limits scale linearly.
    let naval = &Criteria::presets()[0];
    let (hs, by) = report.limiting_height(at(5.0, 90.0), naval);
    assert!(hs.is_finite() && hs > 0.0, "{hs} {by}");
    let mut relaxed = naval.clone();
    relaxed.max_slam_probability = None;
    relaxed.max_wetness_probability = None;
    let roll_only = Criteria {
        max_roll_deg: Some(4.0),
        ..Criteria::default()
    };
    let double = Criteria {
        max_roll_deg: Some(8.0),
        ..Criteria::default()
    };
    let c = at(5.0, 90.0);
    let ratio = report.limiting_height(c, &double).0 / report.limiting_height(c, &roll_only).0;
    assert!((ratio - 2.0).abs() < 1e-9);
    // Motions at the limiting height sit exactly on the limit.
    let lim = report.limiting_height(c, &roll_only).0;
    assert!((report.motions(c, lim).roll_deg - 4.0).abs() < 1e-9);
    // Outlook: the sea builds from calm to a gale; operability is lost.
    let mut forecast = Forecast::default();
    for (i, hs) in [0.3, 1.0, 2.5, 4.5].into_iter().enumerate() {
        forecast.hours.push(Conditions {
            time: format!("2026-11-23T0{i}:00"),
            wave_height: Some(hs),
            wave_period: Some(5.0 * 0.9),
            wave_direction: Some(0.0),
            ..Default::default()
        });
    }
    // Course east with waves from the north: beam seas.
    let outlook = report.outlook(&forecast, 90.0, naval);
    assert_eq!(outlook[0].operable, Some(true), "{:?}", outlook[0]);
    assert_eq!(outlook[3].operable, Some(false), "{:?}", outlook[3]);
    assert!(!outlook[3].limited_by.is_empty());
    // Same table on any number of threads.
    assert_eq!(
        serde_json::to_string(&report).unwrap(),
        serde_json::to_string(&run(1)).unwrap()
    );
}
