# Contributing

Start with docs/BUILDING.md and run the release tests, formatting and strict Clippy.
Keep the headless engine independent from the Windows studio. New physical/sensor
behaviour needs a saved-scene representation, bounds/units, deterministic input,
an analytic or independent comparison, a failure-range statement and documentation.
Do not add a browser/interpreter runtime. Keep UI changes consistent in both themes
and expose named native accessibility controls with keyboard equivalents.

Use a small reproducer and the issue templates for bugs. Do not attach personal,
confidential or restricted project/data files; substitute a minimal synthetic scene.
Include engine version/build, Windows version, CPU, dt/seed and reproduction steps.
Performance reports include resolution, quality, counts and independent stage timings.
Do not infer scientific accuracy from plausible pictures or a passing unit test.

Pull requests describe the concrete problem, resulting behaviour and relevant
validation. Project/run compatibility is reviewed before merging format changes.
Distribution changes must pass install, update, rollback and corrupted-payload tests.
License new assets explicitly and update dependency notices when Cargo.lock changes.
