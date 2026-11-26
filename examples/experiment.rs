//! Headless project → experiment → portable dataset, using the studio's scene model.
use std::{path::Path, sync::atomic::AtomicBool};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "--verify") {
        let run = pelagic::experiment::Recording::read(std::fs::File::open(
            args.get(2).ok_or("run path")?,
        )?)?;
        pelagic::experiment::verify(&run, &AtomicBool::new(false))?;
        println!("Verified {} truth samples", run.frames.len());
        return Ok(());
    }
    let project = args
        .get(1)
        .ok_or("usage: experiment <project.pelagic> <output directory> [seconds]")?;
    let folder = args.get(2).ok_or("output directory")?;
    let seconds = args
        .get(3)
        .map(|s| s.parse::<f64>())
        .transpose()?
        .unwrap_or(10.0);
    let scene = pelagic::scene::Scene::read(Path::new(project))?;
    let run =
        pelagic::experiment::batch(&scene, seconds, &AtomicBool::new(false), |tick, total| {
            if tick % 600 == 0 {
                println!("{tick}/{total} ticks");
            }
        })?;
    pelagic::experiment::verify(&run, &AtomicBool::new(false))?;
    if args.iter().any(|a| a == "--camera") {
        pelagic::dataset::camera(Path::new(folder), &run, &AtomicBool::new(false), |_, _| {})?;
    } else {
        pelagic::experiment::export_bundle(Path::new(folder), &run)?;
    }
    println!(
        "Exported and verified {} samples to {folder}",
        run.frames.len()
    );
    Ok(())
}
