//! Experiments panel: live recording, headless batch runs, recorded-truth
//! playback, re-simulation checks and dataset/camera exports. Ported from the
//! legacy editor's Run actions; all work goes through the same SDK calls
//! (`pelagic::experiment`, `pelagic::dataset`) and runs on background jobs.
use super::{FlyCam, Selection, State, native, theme};
use crate::jobs::{Event, Job, Output};
use crate::project::{Document, Scene};
use egui::{RichText, Ui};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{SyncSender, TryRecvError},
};

const RUN_FILTER: &str = "Pelagic recording\0*.pelagic-run\0";

impl State {
    /// Polls the background task and recording backups once per frame.
    pub(super) fn poll_background(&mut self) {
        let mut finished = None;
        if let Some(job) = &self.job {
            loop {
                match job.events.try_recv() {
                    Ok(Event::Progress(text)) => self.job_status = text,
                    Ok(Event::Finished(result)) => {
                        finished = Some(result);
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        finished = Some(Err("Background task stopped unexpectedly.".into()));
                        break;
                    }
                }
            }
        }
        if let Some(result) = finished {
            self.job = None;
            self.job_status.clear();
            match result {
                Ok(Output::Run(run)) => {
                    crate::editor::keep_run(&mut self.model, Arc::new(*run));
                    self.model.playback = None;
                    self.log("Run ready. Use Replay to inspect the recorded truth.");
                }
                Ok(Output::Notice(text)) => self.log(text),
                Ok(Output::Campaign(done)) => {
                    let (report, scene) = *done;
                    self.campaign_finished(report, scene);
                }
                Ok(Output::Trials(report)) => self.trials_finished(*report),
                Ok(Output::Seakeeping(report)) => self.seakeeping_finished(*report),
                Ok(Output::Sar(report)) => self.sar_finished(*report),
                Ok(Output::Survey(report)) => self.survey_finished(*report),
                Ok(Output::Colregs(results)) => self.colregs_finished(results),
                Ok(Output::Spill(s)) => self.spill_finished(*s),
                Ok(Output::Inspection(r)) => self.inspection_finished(*r),
                Ok(_) => {}
                Err(e) => self.log(format!("Task failed: {e}")),
            }
        }
        let mut pending = vec![];
        for receiver in std::mem::take(&mut self.model.pending_saves) {
            match receiver.try_recv() {
                Ok(Ok(path)) => self.log(format!("Recording backed up: {}", path.display())),
                Ok(Err(failure)) => {
                    self.log(format!(
                        "Run backup failed: {}. Kept in memory; use Experiments > Retry backups.",
                        failure.message
                    ));
                    self.model.failed_runs.push(failure.run);
                }
                Err(TryRecvError::Empty) => pending.push(receiver),
                Err(TryRecvError::Disconnected) => {
                    self.log("Run backup worker stopped; save the loaded recording manually.")
                }
            }
        }
        self.model.pending_saves = pending;
    }

    pub(super) fn background_busy(&self) -> bool {
        self.job.is_some() || !self.model.pending_saves.is_empty()
    }

    pub(super) fn start_job(
        &mut self,
        label: &str,
        work: impl FnOnce(Arc<AtomicBool>, SyncSender<Event>) -> Result<Output, String> + Send + 'static,
    ) {
        if self.job.is_some() {
            self.log("A task is running; cancel it or wait for it to finish.");
            return;
        }
        self.job = Some(Job::start(work));
        self.job_status = label.into();
    }

    /// Experiments must not start while recordings are waiting to be saved.
    fn backups_clear(&mut self) -> bool {
        if !self.model.failed_runs.is_empty() || self.model.pending_saves.len() >= 4 {
            self.log("Finish or retry recording backups before starting another experiment.");
            return false;
        }
        true
    }

    pub(super) fn start_recording(&mut self) {
        if !self.backups_clear() {
            return;
        }
        let scene = self.scene().clone();
        if let Some(session) = self.model.session.as_mut()
            && session.capture.is_none()
        {
            session.start_recording(&scene);
            self.log("Recording truth and sensor observations.");
        }
    }

    pub(super) fn stop_recording(&mut self) {
        crate::editor::archive_run(&mut self.model);
        self.log("Recording kept and queued for backup in the Runs folder.");
    }

    pub(super) fn start_batch(&mut self) {
        if !self.backups_clear() {
            return;
        }
        let scene = self.scene().clone();
        let seconds = self.model.batch_seconds.clamp(0.1, 120.0);
        self.start_job("Running headless batch…", move |cancel, send| {
            let run = pelagic::experiment::batch(&scene, seconds, &cancel, |tick, total| {
                let _ = send.send(Event::Progress(format!("Batch {tick}/{total} ticks")));
            })?;
            Ok(Output::Run(Box::new(run)))
        });
    }

    fn open_recording(&mut self) {
        let Some(path) = native::pick_file(
            self.hwnd,
            false,
            "Open recording",
            RUN_FILTER,
            "pelagic-run",
        ) else {
            return;
        };
        self.start_job("Opening recording…", move |_, _| {
            let run = pelagic::experiment::Recording::read(
                std::fs::File::open(path).map_err(|e| e.to_string())?,
            )?;
            let scene = serde_json::from_str(&run.scene_json).map_err(|e| e.to_string())?;
            Scene::from_value(&scene)?;
            Ok(Output::Run(Box::new(run)))
        });
    }

    /// Installs the recording's scene and shows its first truth sample.
    pub(super) fn replay_recording(&mut self) {
        let Some(run) = self.model.run.clone() else {
            self.log("Record, open or batch a run first.");
            return;
        };
        let scene = serde_json::from_str(&run.scene_json)
            .map_err(|e| e.to_string())
            .and_then(|v| Scene::from_value(&v));
        let scene = match scene {
            Ok(scene) => scene,
            Err(e) => return self.log(format!("Recording scene is invalid: {e}")),
        };
        if !self.confirm_discard("replaying a recording") {
            return;
        }
        self.cam = FlyCam::from_scene(&scene);
        crate::editor::install(&mut self.model, Document::new(scene));
        self.model.run = Some(run);
        self.select(Selection::None);
        match crate::editor::replay(&mut self.model, 0) {
            Ok(()) => self.log("Recorded truth playback. The live simulation is paused."),
            Err(e) => self.log(e),
        }
        self.view_dirty = true;
    }

    pub(super) fn scrub(&mut self, index: usize) {
        if let Err(e) = crate::editor::replay(&mut self.model, index) {
            self.log(e);
        }
        self.view_dirty = true;
    }

    fn return_to_live(&mut self) {
        self.model.playback = None;
        if let Some(session) = self.model.session.as_mut() {
            session.show_live();
        }
        self.view_dirty = true;
    }

    fn export_folder(&mut self, camera: bool) {
        let Some(run) = self.model.run.clone() else {
            return;
        };
        let Some(parent) = native::pick_folder(self.hwnd, "Choose a folder for the dataset") else {
            return;
        };
        let folder = std::path::Path::new(&parent).join(format!(
            "Pelagic-dataset-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ));
        let label = if camera {
            "Exporting camera dataset…"
        } else {
            "Exporting dataset bundle…"
        };
        self.start_job(label, move |cancel, send| {
            if camera {
                pelagic::dataset::camera(&folder, &run, &cancel, |current, total| {
                    let _ = send.send(Event::Progress(format!("Camera {current}/{total}")));
                })?;
            } else {
                pelagic::experiment::export_bundle(&folder, &run)?;
            }
            Ok(Output::Notice(format!(
                "Dataset exported: {}",
                folder.display()
            )))
        });
    }

    fn save_recording(&mut self) {
        let Some(run) = self.model.run.clone() else {
            return;
        };
        let Some(path) =
            native::pick_file(self.hwnd, true, "Save recording", RUN_FILTER, "pelagic-run")
        else {
            return;
        };
        self.start_job("Saving recording…", move |_, _| {
            pelagic::experiment::write_atomic_with_replace(
                std::path::Path::new(&path),
                &run,
                true,
            )?;
            Ok(Output::Notice(format!("Recording saved: {path}")))
        });
    }

    fn verify_recording(&mut self) {
        let Some(run) = self.model.run.clone() else {
            return;
        };
        self.start_job("Re-simulating recording…", move |cancel, _| {
            pelagic::experiment::verify(&run, &cancel)?;
            Ok(Output::Notice(
                "Re-simulation matches every recorded truth sample on this build.".into(),
            ))
        });
    }

    fn export_mounted_camera(&mut self) {
        let Some(path) = native::pick_file(
            self.hwnd,
            true,
            "Export mounted camera",
            "PNG image\0*.png\0",
            "png",
        ) else {
            return;
        };
        match crate::editor::export_camera(&self.model, std::path::Path::new(&path)) {
            Ok(()) => self.log(format!("Camera image and calibration saved: {path}")),
            Err(e) => self.log(format!("Camera export failed: {e}")),
        }
    }

    pub(super) fn experiments(&mut self, ui: &mut Ui) {
        if self.model.batch_seconds <= 0.0 {
            self.model.batch_seconds = 10.0;
        }
        // Two columns: recording/playback tools and the test campaign.
        ui.columns(2, |columns| {
            let [left, right] = columns else {
                unreachable!()
            };
            egui::ScrollArea::vertical()
                .id_salt("experiments-runs")
                .show(left, |ui| self.run_tools(ui));
            egui::ScrollArea::vertical()
                .id_salt("experiments-campaign")
                .show(right, |ui| self.campaign_panel(ui));
        });
    }

    fn run_tools(&mut self, ui: &mut Ui) {
        {
            ui.add_space(4.0);
            if let Some(job) = &self.job {
                let cancel = job.cancel.clone();
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(&self.job_status);
                    if ui.button("Cancel task").clicked() {
                        cancel.store(true, Ordering::Relaxed);
                        self.job_status = "Cancelling…".into();
                    }
                });
                ui.separator();
            }
            let busy = self.job.is_some();
            let has_session = self.model.session.is_some();

            theme::heading(ui, "Live recording");
            let recording = self.model.session.as_ref().and_then(|s| {
                s.capture
                    .as_ref()
                    .map(|r| (r.frames.len(), s.recording_full))
            });
            ui.horizontal(|ui| match recording {
                None => {
                    if ui
                        .add_enabled(has_session, egui::Button::new("⏺ Record"))
                        .on_hover_text(
                            "Capture truth and sensor observations while the simulation runs",
                        )
                        .clicked()
                    {
                        self.start_recording();
                    }
                    ui.label(RichText::new("Not recording").color(super::theme::MUTED));
                }
                Some((samples, full)) => {
                    if ui.button("⏹ Stop recording").clicked() {
                        self.stop_recording();
                    }
                    ui.label(format!(
                        "{samples} samples{}",
                        if full { " · limit reached" } else { "" }
                    ));
                }
            });
            ui.add_space(6.0);

            theme::heading(ui, "Headless batch");
            ui.horizontal(|ui| {
                ui.label("Duration");
                let drag = ui.add(
                    egui::DragValue::new(&mut self.model.batch_seconds)
                        .speed(0.5)
                        .range(0.1..=120.0)
                        .suffix(" s"),
                );
                crate::studio::a11y::name(&drag, "Batch duration");
                if ui
                    .add_enabled(!busy && has_session, theme::primary("▶ Run batch"))
                    .on_hover_text("Simulate the current scene off-screen and keep the recording")
                    .clicked()
                {
                    self.start_batch();
                }
            });
            ui.add_space(6.0);

            theme::heading(ui, "Loaded recording");
            match self.model.run.clone() {
                None => {
                    ui.label(
                        RichText::new("None yet. Record, run a batch or open a .pelagic-run file.")
                            .color(super::theme::MUTED),
                    );
                }
                Some(run) => {
                    let samples = run.frames.len();
                    let span = match (run.frames.first(), run.frames.last()) {
                        (Some(a), Some(b)) => b.truth.time - a.truth.time,
                        _ => 0.0,
                    };
                    ui.label(format!("{samples} samples · {span:.1} s"));
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!busy, egui::Button::new("⏵ Replay"))
                            .clicked()
                        {
                            self.replay_recording();
                        }
                        if self.model.playback.is_some() && ui.button("Return to live").clicked() {
                            self.return_to_live();
                        }
                    });
                    if let Some(mut index) = self.model.playback {
                        let last = samples.saturating_sub(1);
                        let time = run.frames.get(index).map_or(0.0, |f| f.truth.time);
                        ui.spacing_mut().slider_width = (ui.available_width() - 140.0).max(120.0);
                        let slider = ui.add(
                            egui::Slider::new(&mut index, 0..=last)
                                .text(format!("t = {time:.2} s")),
                        );
                        crate::studio::a11y::name(&slider, "Playback sample");
                        if slider.changed() {
                            self.scrub(index);
                        }
                    }
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(!busy, egui::Button::new("💾 Save as…"))
                            .clicked()
                        {
                            self.save_recording();
                        }
                        if ui
                            .add_enabled(!busy, egui::Button::new("✔ Verify re-simulation"))
                            .on_hover_text(
                                "Re-run the recorded commands and compare every truth sample",
                            )
                            .clicked()
                        {
                            self.verify_recording();
                        }
                        if ui
                            .add_enabled(!busy, egui::Button::new("📦 Export dataset…"))
                            .on_hover_text("Truth, observations and metadata as CSV/JSON")
                            .clicked()
                        {
                            self.export_folder(false);
                        }
                        if ui
                            .add_enabled(!busy, egui::Button::new("📷 Export camera dataset…"))
                            .on_hover_text("Mounted-camera images for up to 120 samples")
                            .clicked()
                        {
                            self.export_folder(true);
                        }
                    });
                }
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!busy, egui::Button::new("📂 Open recording…"))
                    .clicked()
                {
                    self.open_recording();
                }
                if ui
                    .add_enabled(has_session, egui::Button::new("📷 Export mounted camera…"))
                    .on_hover_text("Current sensor-camera image with pinhole calibration")
                    .clicked()
                {
                    self.export_mounted_camera();
                }
            });
            if !self.model.failed_runs.is_empty() {
                ui.add_space(6.0);
                let count = self.model.failed_runs.len();
                if ui
                    .button(format!("⟳ Retry backups ({count} failed)"))
                    .clicked()
                {
                    for run in std::mem::take(&mut self.model.failed_runs) {
                        crate::editor::keep_run(&mut self.model, run);
                    }
                    self.log("Retrying recording backups.");
                }
            }
            if !self.model.pending_saves.is_empty() {
                ui.label(
                    RichText::new(format!(
                        "Backing up {} recording(s)…",
                        self.model.pending_saves.len()
                    ))
                    .color(super::theme::MUTED),
                );
            }
        }
    }
}
