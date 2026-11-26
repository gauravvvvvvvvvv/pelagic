# Five worked projects

Launch `Pelagic.exe`. Use Open project to choose a supplied project in `Examples`.
For the first session use Low quality. The 4-second logo/ocean cue ends at Projects;
sound and theme preferences persist. Windows animation preferences enable reduced
motion. Tab/Shift-Tab move focus, Enter activates, and Escape cancels field editing.
Fields select their contents on entry; type a value and press Enter to commit.
Ctrl+S saves; Ctrl+Z/Ctrl+Y undo/redo committed authoring edits.

The studio separates scene authoring from a temporary run. A physical edit or
Reset starts the preview again. Save project stores configuration; Save recording
stores truth and observations. Stopped recordings are backed up to
`%LOCALAPPDATA%\Pelagic\Runs`. Keep that distinction when comparing results.

## 1. Surface sea-state exploration

Open `v5-surface.pelagic`. Surface shows a prescribed spectral sea and a floating
reference boat beside the harbour. Play, pause, Step and Reset use fixed simulation
ticks. In Ocean environment choose the Waves tab, change significant wave height,
peak period and direction, and commit. Save As a new project before experimenting.
Quality changes resolution only. Surface, Underwater and Section are complementary
views of the same sampled environment. A wave-height input is a spectral parameter,
not a guarantee of the largest individual crest in a finite record.

Use right drag to orbit, wheel to zoom, W/A/S/D to pan, Q/E to change elevation and
F to frame the scene. Simulation pauses while minimized. Headless batch is the
route for uninterrupted computation without a viewport.

## 2. Underwater vehicle operation

Open `v5-robotics.pelagic`. It contains an ROV, surface boat, drifter, 24 fish and 64
tracers. Choose Entities, select the reference vehicle, and inspect Transform,
Body/actuators and Controller. Adjust depth hold, heading or force within the
displayed units. Play and compare resulting motion with the configured current.
Depth/heading hold are reference PD controllers, with saturation and actuator lag.

Add/manage offers additional bodies, duplication, deletion and geometry import.
An imported OBJ is embedded; physical contact and sonar use the declared body box.
Match that box to the imported geometry before interpreting a clearance test.
Undo a changed body parameter; observe that physical edits reset the preview.

## 3. School environment

Open `v5-ecology.pelagic`. Marine life contains Population and Behaviour settings.
Change count, speed, radius, preferred depth, separation and vehicle avoidance.
The source seed is saved, so reset yields repeatable initial conditions. Add a
vehicle to inspect an avoidance response. These are illustrative pelagic fish;
the parameters are not calibrated biological species measurements. Steering
separation is not a guaranteed collision-free packing constraint.

## 4. Sensor experiment, recording and export

Open `v5-sensors.pelagic`. Sensors has sonar configuration, delivery timing and
motion/noise controls. More views selects sonar, results or the mounted camera.
Sonar reports distinct truth and noisy ranges; delayed measurements retain both
sample and availability timestamps. Results plot recorded depth and speed.

In Experiments start recording, Play for a few seconds, then stop recording.
Run actions can Save loaded recording, Replay, Verify re-simulation and Return to
live scene. Save then reopen a `.pelagic-run`; verify it again. Replay is read-only
saved truth; it does not drive the live physics world. Changing the project while
a dataset is being prepared does not silently replace that dataset's scene.

Export dataset bundle to a **new** directory for JSON, source scene and labelled
CSV files. Export camera dataset adds bounded PPM images and calibration metadata.
The CLI reproduces the same batch from the portable package:

```powershell
./SDK/experiment.exe ./Examples/v5-sensors.pelagic ./MySensorDataset 5 --camera
./SDK/experiment.exe --verify ./MySensorDataset/experiment.pelagic-run
```

Use a new destination on each export. `INCOMPLETE.txt` means a cancelled/failed
export must not be treated as complete. If recording backup fails, the app pauses
and retains the recording. Free space/fix access, then choose Experiments > Run
actions > Retry failed backups before closing. It never dismisses this error as a
successful save. Batches, verification and camera export expose cancellation.

## 5. Real-region exploration

Open `v5-goa.pelagic`. This is a bounded regional scene resampled from NOAA ETOPO1,
not a newly measured survey. F frames its full extent; regional zoom supports its
kilometre scale. Atlas/data reports source, licence, retrieval date, resolution and
geographic origin. The overview atlas is intentionally lower-resolution context.

Import supported latitude/longitude/elevation ERDDAP CSV, local height-grid CSV or
ESRI ASCII terrain through the atlas/data tools. Current-grid import expects a
complete regular XYZ/time lattice in SI units. Missing/duplicate samples and
unsupported geometry fail explicitly. Imported data is embedded when you Save.
Preserve source attribution and distinguish measured observations from composites
and generated fields. Do not infer metre-level clearance from kilometre terrain.

## Developer workflows

`SDK/engine_host.exe SDK/pelagic.dll` demonstrates the versioned C ABI. Run
`SDK/mavlink_loopback.exe` for an isolated local UDP smoke test. The adapter supports
only documented MAVLink 1 messages and has a command watchdog; it is not a full
autopilot simulator. Rust hosts use `Scene`, `World`, `Suite`, `Recording` and the
shared renderer directly. See SDK.md, PROJECT-FORMAT.md and MODEL-VALIDATION.md.
