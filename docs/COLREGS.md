# Ship handling and the collision regulations

Status: 24 November 2026.

Pelagic puts a surface vessel, with its own physics, into encounters with
other vessels under the International Regulations for Preventing
Collisions at Sea (COLREGs 1972, Part B). You steer it with helm and
engine orders on a radar plot, or hand over to a reference
collision-avoidance autopilot, and get a rule-by-rule debrief. The same
engine runs batches for testing autonomous-vessel controllers. It is a
training and test aid for navigation safety.

## What it models

- **Encounter geometry:** closest point of approach (CPA) and time to it
  (TCPA) from positions and velocities; bearings relative to the bow.
- **Roles**, fixed when risk of collision is first found (Rule 13(d)):
  - Rule 13, overtaking: coming up from more than 22.5° abaft the other's
    beam. It applies notwithstanding Rule 18.
  - Rule 14, head-on: each sees the other within ±10° of ahead
    (configurable); both alter to starboard.
  - Rule 15, crossing: the vessel with the other on her starboard side
    gives way and avoids crossing ahead.
  - Rule 17, stand-on: keep course and speed; may act when the give-way
    vessel is not acting; must act when collision cannot be avoided by the
    give-way vessel alone; in crossing, must not alter to port for a vessel
    on her own port side.
  - Rule 18: a power-driven vessel keeps out of the way of sailing,
    fishing, restricted and not-under-command vessels.
- **Own ship** is the selected vehicle in the scene's sea and current, run
  by the shared helm (heading loop sized from its rudder and thruster
  authority, speed control with astern thrust). **Targets** move
  kinematically, turning at a rate set by their length, and can carry
  scripted orders, such as a give-way vessel that alters at a given time.

## Encounters

Standard set, scaled to own ship's service speed (65 % thrust in calm
water) and length:

1. **Head-on.** Both vessels alter to starboard.
2. **Crossing, give way.** A vessel on your starboard bow holds on.
3. **Crossing, stand on.** A vessel on your port bow gives way.
4. **Crossing, stand on: the other vessel does not act.** Tests Rule 17.
5. **Overtaking** a slower vessel.
6. **Being overtaken** by a faster vessel, which keeps clear.
7. **Fishing vessel crossing from port.** Rule 18 makes you give way.
8. **Head-on and crossing together.**

Random encounters (seeded) place 1–4 vessels of mixed kinds on or near
collision courses; they keep their course and speed.

## Thresholds (configurable interpretations)

The Regulations give no numbers. The defaults below scale with own ship's
length:

| Setting | Default |
|---|---|
| Safe passing distance | 15 × length (50 m – 1 NM) |
| Give-way acts before | 6 × safe distance (or promptly once risk arises) |
| Stand-on may act inside | 4 × safe distance |
| Stand-on must act inside | 2 × safe distance |
| Substantial alteration | ≥ 30°, or an order to half speed or less |
| Head-on sector | ±10° |

## Reference autopilot

The reference autopilot re-plans every 2 s by searching courses (±90° in 5°
steps around the course when avoidance began) and speeds (full, half and
quarter). Each candidate is scored on:

- predicted CPA with every vessel;
- deviation from the route;
- COLREG penalties: turning to port when giving way or standing on in a
  crossing, crossing ahead of a stand-on vessel, and small (< 30°)
  alterations;
- steadiness, so it holds one clear alteration instead of a succession of
  small ones.

A stand-on vessel keeps course and speed until Rule 17 allows or requires
action. After a vessel is finally past and clear (opening, abaft the beam
and well off), the autopilot returns to its route without closing any
vessel nearby.

**Results with the example workboat (15 kn):** all 8 standard encounters
pass every check, and so do 46 of 48 random ones. The two failures are
honest edge cases:

- two vessels on the port side that both fail to give way at speed (passed
  at 61 m);
- an overtaking situation that arose from an earlier manoeuvre (action
  judged small and late).

## In the editor (Ship Handling tab)

1. Select a surface vessel in the Outliner; it becomes own ship.
2. Choose an encounter and the safe passing distance, then press **▶ Start
   encounter**. You have the helm:
   - **Helm** buttons (5° and 20°) or ←/→ keys alter course;
   - **Engines** (Stop, Slow, Half, Full) or ↑/↓ keys change speed;
   - ×1, ×4 or ×16 speeds up time.
3. The radar plot is north-up with own ship at the centre. It shows range
   rings, one-minute true vectors, 10-second trails, the safe-distance ring
   and the planned route. Any vessel with a CPA inside the safe distance
   turns red, with its CPA line. The table lists bearing, range, CPA, TCPA
   and the situation under the rules.
4. Tick **COLREGs autopilot has the helm** to hand over at any time.
5. At the end, the debrief lists for each vessel:
   - the situation and the closest range;
   - whether action was early (Rules 8(a), 16) and substantial (Rule 8(b));
   - for head-on, whether you altered to starboard (Rule 14);
   - for crossing give-way, whether you passed astern (Rule 15);
   - for stand-on, whether you kept course and speed or acted correctly
     (Rule 17).

**Run all with autopilot** runs the 8 standard and 12 random encounters in
the background; **Export…** saves the results.

## Command line

```
Pelagic.exe --colregs <vehicle.pelagic-vehicle.json> <out-folder>
  [--random=N] [--seed=N] [--safe=M] [--only=standard|random] [--threads=N]
```

Writes `summary.txt`, `colregs.json` and `track-<encounter>.csv` (own ship
and targets every second). Exit code 0 when every check passed, 2 when any
failed, 1 on error. 56 encounters take about 7 s on a laptop.

## Limits

- Vessels "in sight of one another" (Section II). Restricted visibility
  (Rule 19), narrow channels and traffic separation schemes (Rules 9, 10)
  and sound and light signals are not modelled.
- Targets are kinematic (no physics), with constant turn rate and
  acceleration; their own collision avoidance is scripted, not reactive.
- Open water: no land, depth or other hazards in the encounter area.
- Thresholds are interpretations for training; a qualified examiner's
  judgement of "early", "substantial" and "safe" depends on the vessels and
  the situation.
