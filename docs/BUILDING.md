# Building and checking Pelagic

Windows x64 is the supported desktop platform. The engine is a normal Rust library;
other targets can build the headless core, but have not passed desktop acceptance.

Install current stable Rust, the `stable-x86_64-pc-windows-gnu` toolchain, and LLVM-MinGW
or MinGW binutils. Put the toolchain's `dlltool.exe` on this shell's PATH. Pelagic uses
Rust's bundled LLD to link Windows import libraries consistently. The application
ships no compiler, scripting interpreter, browser, WebView or downloaded runtime.

From the repository root in PowerShell:

```powershell
. ./scripts/Initialize-Toolchain.ps1
cargo +stable-x86_64-pc-windows-gnu fetch --locked
cargo +stable-x86_64-pc-windows-gnu fmt --all --check
cargo +stable-x86_64-pc-windows-gnu clippy --workspace --all-targets --offline -- -D warnings
cargo +stable-x86_64-pc-windows-gnu test --workspace --release --offline
cargo +stable-x86_64-pc-windows-gnu build --release --workspace --bins --examples --offline
./scripts/Build-Package.ps1
```

The release tests include an hour of neutral-body simulation and ten minutes of
schooling, executed faster than real time. Do not replace them with debug timing.
`acceptance.exe NEW-REPORT.json 36000` measures three workloads and allocation
growth. Run it without compilation or other heavy jobs competing for the CPU.
`pelagic-desktop.exe --benchmark-ui NEW-REPORT.json` measures painting separately.
Run `scripts/Test-Accessibility.ps1` with Windows PowerShell 5.1 to exercise this
app's UI Automation controls in an isolated test session.

`Cargo.lock` freezes dependencies. `build.rs` produces a provenance fingerprint of
the engine sources/compiler/target/profile, not a cryptographic package signature.
Package integrity uses SHA-256. Two byte-identical executables are not promised
across compiler versions or Windows linker timestamps. Recorded truth plays back
exactly; bitwise re-simulation is checked for the recorded build and parameters.

The local GNU import generator was obtained from the upstream
[LLVM-MinGW release](https://github.com/mstorsjo/llvm-mingw/releases/tag/20261104).
It is a build tool, not shipped code. A broken GNU import library discovered by the
release harness led to choosing LLD explicitly. Do not remove this setting without
running the native desktop, SDK host and accessibility tests.
