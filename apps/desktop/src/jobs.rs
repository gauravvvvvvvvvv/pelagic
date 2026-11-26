use crate::project::Scene;
use pelagic::experiment::Recording;
use std::sync::{
    Arc,
    atomic::AtomicBool,
    mpsc::{Receiver, SyncSender},
};
pub enum Output {
    Gpu(Box<pelagic_render_gpu::GpuRenderer>, u64),
    Run(Box<Recording>),
    Notice(String),
    /// Finished test campaign and the scene it ran on.
    Campaign(Box<(pelagic::campaign::Report, Scene)>),
    /// Finished standard trials.
    Trials(Box<pelagic::trials::Report>),
    /// Finished seakeeping response table.
    Seakeeping(Box<pelagic::seakeeping::Report>),
    /// Finished search-and-rescue drift and plans.
    Sar(Box<pelagic::sar::Report>),
    /// Finished survey plan (and physics run).
    Survey(Box<pelagic::survey::Report>),
    /// Encounters run with the COLREGs autopilot.
    Colregs(Vec<pelagic::colregs::Outcome>),
    /// Finished oil spill drift.
    Spill(Box<pelagic::spill::Spill>),
    /// Finished inspection plan (and flight).
    Inspection(Box<pelagic::inspection::Report>),
    Scene {
        before: Box<Scene>,
        after: Box<Scene>,
    },
}
pub enum Event {
    Progress(String),
    Finished(Result<Output, String>),
}
pub struct Job {
    pub events: Receiver<Event>,
    pub cancel: Arc<AtomicBool>,
}
impl Job {
    pub fn start(
        work: impl FnOnce(Arc<AtomicBool>, SyncSender<Event>) -> Result<Output, String> + Send + 'static,
    ) -> Self {
        let (send, events) = std::sync::mpsc::sync_channel(8);
        let cancel = Arc::new(AtomicBool::new(false));
        let token = cancel.clone();
        std::thread::spawn(move || {
            let output = std::panic::catch_unwind(std::panic::AssertUnwindSafe(||work(token, send.clone()))).unwrap_or_else(|_|Err("Background task failed unexpectedly. Inspect partial exports before using them.".into()));
            let _ = send.send(Event::Finished(output));
        });
        Self { events, cancel }
    }
}
pub fn goa() -> Result<Scene, String> {
    let (grid, latitude, longitude) =
        pelagic::data::geographic_csv(include_str!("../../../examples/data/goa-etopo1.csv"))
            .map_err(str::to_string)?;
    let mut scene = Scene {
        name: "Goa offshore / NOAA ETOPO1".into(),
        terrain: 3,
        grid: Some(Arc::new(grid)),
        structure: false,
        bottom_depth: 100.0,
        camera: [-7000.0, -10000.0, 6000.0],
        target: [0.0, 0.0, 0.0],
        ..Default::default()
    };
    scene.provenance = pelagic::data::Provenance {
        source: "NOAA ETOPO1 Ice Surface / Goa offshore".into(),
        source_url: "https://coastwatch.pfeg.noaa.gov/erddap/griddap/etopo180.html".into(),
        licence: "NOAA public-domain dataset; attribution retained".into(),
        retrieved_utc: "2026-11-14".into(),
        resolution: "1 arc-minute source; ~1.85 km local resampling".into(),
        measured: false,
        classification: "resampled composite terrain".into(),
        origin_latitude: latitude,
        origin_longitude: longitude,
    };
    scene.validate()?;
    Ok(scene)
}
