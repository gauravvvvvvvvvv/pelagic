# Hull and harbour inspection

Status: 24 November 2026.

Pelagic plans camera passes over the underwater surfaces of the harbour
pilings or a vessel's hull, and judges how much of each surface is imaged
sharply enough. It does this on the plan and when a hovering vehicle (an
ROV) flies the plan with its own physics in the scene's sea and current.

## Method

- **Targets:**
  - the scene's harbour pilings (the deck is kept as an obstacle);
  - any surface vessel's hull below its hydrostatic waterline (bottom and
    four sides).

  Each surface is a rectangle split into patches.
- **Image quality:**
  - **Sharpness:** ground sample distance, GSD = range × pixel angle ÷
    cos(incidence).
  - **Contrast:** falls as exp(−c·range) with the water's beam attenuation
    c, and a minimum contrast sets the longest useful range.
  - **Viewing angle:** incidence is limited (default 60°).
- **Stand-off:** 85 % of the shorter of the GSD range and the visibility
  range. It is kept outside a clearance, which is widened to the vehicle's
  size plus 0.4 m when a vehicle flies the plan. If the clearance wins, the
  plan says the images will not meet the requirement everywhere.
- **Passes:**
  - They run along each face's longer side, the frame's wide axis across
    them.
  - Spacing is the part of the frame that still meets the GSD, contrast and
    incidence limits, less the overlap.
  - Faces are taken nearest first, and transits route around the inflated
    structure, staying underwater and above the seabed.
- **Coverage:** a frame images a patch when all of these hold:
  - the patch is inside the frame;
  - it faces the camera within the incidence limit;
  - no other part of the structure hides it;
  - it meets the GSD and contrast limits.

  The tab reports the share imaged to spec, the share seen at all, and the
  best GSD reached.
- **Flying it:**
  - The vehicle tracks a target point moving along the path at survey
    speed, under PID control on position (with integral action only near
    the target point), with its heading held towards the surface.
  - The camera is gimbal-stabilised on the planned view direction, and
    frames (2 Hz) are taken from the vehicle's real pose.
  - The target point waits for the vehicle, but moves on after 30 s without
    progress, and the skipped section is reported.
  - The report gives the position error RMS on the passes and the least
    clearance; a negative clearance means the vehicle touched the
    structure.

## Findings while testing

- The first plan put the lowest passes below the seabed beside the
  pilings: the demo seabed rises around them. The ROV sat on the bottom
  and the run stalled at 12 %. Viewpoints and routing corners are now kept
  a clearance above the local seabed.
- An integral term that wound up during long vertical transits pushed the
  ROV into a piling. Integration now runs only near the target point.
- **Results with the reference ROV (12 kg, 40 N thrust) on the demo
  pilings:**
  - 1 mm/px in calm water: 97 % imaged to spec, 0.15 m RMS position error.
  - In 1.2 m waves: 96 %, but it brushed a piling near the surface, as a
    small ROV would.

## In the editor (Inspection tab)

1. Choose the target (Harbour pilings, or a vessel's hull), the required
   sharpness (mm per pixel), the camera (field of view, image size),
   overlap, speed and clearance, and the water clarity: the scene's, or
   your own attenuation and least contrast. The tab shows the stand-off
   these give.
2. **▶ Plan inspection** for the planned coverage. Select an ROV, then
   **▶ Fly with vehicle physics**.
3. The views show:
   - the passes from above and from the side, with the vehicle's track;
   - each face as a tile of patches: blue imaged to spec, brown seen but
     too coarse or dim, orange missed;
   - a table per face.

**Export…** writes `summary.txt`, `inspection.json`, `faces.csv`, `path.csv`
(waypoints with view directions) and `track.csv`.

## Command line

```
Pelagic.exe --inspect <project.pelagic | demo> <out-folder>
  [--target=pilings|NAME] [--vehicle=NAME | --vehicle-file=PATH]
  [--gsd=MM] [--fov=DEG] [--pixels=WxH] [--overlap=PCT] [--contrast=PCT]
  [--attenuation=PER_M] [--speed=MPS] [--standoff=M]
```

Exit code 0 when at least 95 % is imaged to spec, 2 when less, 1 on error.

## Limits

- Surfaces are boxes: no curved hull form, appendages or marine growth.
  The inspected hull is fixed (not moving with the sea).
- Image quality is geometric (GSD, contrast decay, incidence). There is no
  lighting, backscatter, motion blur or focus model, and no simulated
  images of defects.
- The camera is an ideal stabilised gimbal; frames are taken at 2 Hz.
- Contact with the structure is detected from the vehicle's bounding
  radius, a conservative test.
