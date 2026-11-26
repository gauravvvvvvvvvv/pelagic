# Installing, updating, rollback and integrity

Windows x64 is supported. The portable ZIP is also the installation source.
Extract the entire ZIP to a new folder. Running `Pelagic.exe` there is portable.
Running `PelagicSetup.exe` verifies SHA-256 file hashes, copies a versioned payload
to `%LOCALAPPDATA%\Programs\Pelagic`, creates a stable launcher and adds a per-user
Start menu shortcut. No administrator account, Rust, interpreter or browser is
required. `Examples`, `Data`, `Docs` and `SDK` accompany each installed release.

To update, close the installer if already running, extract the newer ZIP and run
its setup. Existing app sessions can finish; a subsequent stable launch selects the
new release. A synced atomic `current.json` selects the version only after payload
verification. `previous.json` retains the previous selection. Projects, preferences,
recordings and recovery under `%LOCALAPPDATA%\Pelagic` are preserved. The app has no
background updater, telemetry, account requirement or download-on-launch.

For rollback, run the **installed** launcher with `--rollback`. It verifies the
previous payload before switching, and leaves running sessions alone. The two
descriptors can be inspected as ordinary JSON. Old release folders are retained
deliberately; remove obsolete versions only after keeping the desired current and
previous versions. Do not put personal projects inside a managed release folder.

```powershell
./PelagicSetup.exe --verify . --quiet --report ./verification.json
./PelagicSetup.exe --destination C:/Some/Empty/Pelagic --quiet --report ./install.json
& "$env:LOCALAPPDATA/Programs/Pelagic/Pelagic.exe" --rollback
```

The destination option omits the Start menu shortcut, which is useful for isolated
installation tests. Setup refuses an occupied destination without an installation
descriptor. An install lock prevents concurrent writers. If setup itself crashes,
close it, remove its stale `install.lock` and retry. A failed incomplete version
folder can be removed after verifying it is not selected in `current.json` or
`previous.json`; original projects remain outside these folders.

To remove the app, close it, delete the Pelagic Start menu shortcut and remove its
installation folder. Keep `%LOCALAPPDATA%\Pelagic` until recovery/recordings have
been backed up. There is no service or machine-wide registry installation.

## Signing decision

This release is **unsigned**. No Authenticode certificate/publisher identity was
provided, and a fake/self-signed identity would not establish publisher trust.
Windows may show an unknown-publisher warning. Distribution is a manually shared
ZIP with a SHA-256 sidecar plus per-file hashes. Hashes detect corruption against
a trusted copy; a manifest delivered beside modified files is not authentication.
For public distribution, publish the ZIP hash through a trusted channel and sign
the executable/setup using the publisher's certificate in the release pipeline.
No certificate, signing service or automatic SmartScreen reputation is claimed.

Package verification runs before installation and again after copying. Project
and run parsers separately validate their own size, schema and finite-value bounds.
The source fingerprint inside scenes/runs identifies the engine calculation; it
does not substitute for the ZIP's cryptographic hash or an Authenticode signature.
