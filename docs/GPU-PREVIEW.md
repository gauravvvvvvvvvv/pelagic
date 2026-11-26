# Native CPU/GPU preview

This is the first implementation milestone of the expanded engine direction.
It establishes a working native graphics path and shared geometry. It does not
complete the expanded backlog or establish Unreal-level visual quality.

## Trying it

Open `Pelagic-next.exe` and open a project. In the workspace, the renderer button
on the main toolbar opens three choices:

- **CPU reference:** the existing CPU renderer; the default.
- **CPU shared geometry:** the new shared scene with approximate CPU shading.
- **Native GPU:** the experimental native GPU preview.

GPU initialization runs as a background job. The selected mode survives scene
edits, undo and reset. A GPU render error returns to CPU rendering with a visible
notice. Wireframe currently selects CPU shared geometry. Automatic tier selection
is not implemented. Previous local executables are backed up under
`output/editor-redesign/previous` when installing the redesigned editor.

To start directly with a supplied project, from the repository in PowerShell:

```powershell
./Pelagic-next.exe --open ./examples/scenes/v5-robotics.pelagic --gpu
```

The application, engine and renderer are Rust. No browser or scripting runtime is
required. A discrete GPU is optional: the native backend can use an integrated GPU
with a supported driver. CPU mode remains available when that is unavailable.

## Implemented

- Immutable shared geometry for terrain, physical wave surfaces, imported OBJ
  meshes, reference vehicles, fish and tracers.
- Native wgpu device creation, HDR opaque lighting, a depth-aware water pass,
  Fresnel reflection, approximate refraction/attenuation and tone mapping.
- Reusable GPU buffers and render targets, bounded dimensions/geometry, resize and
  aligned readback. GPU output is presented through the existing native studio.
- Surface and underwater previews with procedural sand, cosmetic ripples, foam
  and caustics. These visual effects are approximations, not calibrated optics.
- Snapshot checks ensuring rendering does not advance or modify physical truth.
  Graphics quality changes do not silently reduce physical populations.

## Verification and limitations

Local check on 15 November 2026: Intel Iris Xe integrated graphics, Vulkan,
Intel driver 101.7092, 960 × 600, Balanced geometry (34,652 opaque and 33,280 water
triangles). Ten warm fixed-snapshot samples per view produced:

| View | Geometry preparation, first sample | GPU upload/render/readback mean | Maximum of 10 GPU samples |
|---|---:|---:|---:|
| Surface | 9.79 ms | 6.52 ms | 7.53 ms |
| Underwater | 10.40 ms | 6.20 ms | 6.61 ms |

These stages exclude simulation and editor painting. They are not full-app FPS
measurements. Device initialization took 139 ms with a warm driver cache. Full
local evidence is in `output/gpu-milestone/report.json` (engine `caf380ca391a03db`,
GPU source/shader `ef57552d496b8723`). Earlier cold-driver runs were slower.

The isolated native desktop smoke check also passed: GPU selection survived a
fish-count edit and restoration, 1,000 paint calls grew no GDI handles and used
3.37 MiB additional private memory, and the paused two-second sample consumed
0 ms measured CPU time. This short check does not establish long-session stability.

The comparison example renders the same snapshot through both previews. GPU
repeats are checked byte-for-byte within this run; CPU and GPU pixels differ
because their shading differs. It also exercises non-aligned readback dimensions.

Camera sensor and dataset exports still use the established CPU reference renderer.
They do not silently switch to the GPU preview. The core and GPU source/shader
fingerprints are recorded separately in the comparison report.

This preview uses synchronous GPU readback into the existing GDI presentation
path. It does not yet use a direct window swapchain. Geometry preparation is CPU
work and must be included when assessing frame cost. Cold CPU rendering versus
warm GPU rendering is not a fair speedup comparison.

Shadow maps, imported PBR texture maps, anti-aliasing, an authored production asset
set and full material parity remain open. Docking, gizmos, executable node graphs,
game export and experimental learning integration are separate upcoming work.
The current assets still look like prototypes; a new renderer alone cannot turn
them into a finished art direction.

Reproduce the hardware check with a new output directory:

```powershell
. ./scripts/Initialize-Toolchain.ps1
cargo +stable-x86_64-pc-windows-gnu run --release -p pelagic-render-gpu --example compare --offline -- output/my-gpu-check
```

The report includes adapter/driver, source fingerprints, geometry preparation,
GPU upload/render/readback measurements and a first CPU shared-geometry frame.
It is a local diagnostic, not a certified benchmark or cross-machine guarantee.
