# interior

Indoor planning as geometry rather than opinion. Reads a measured room model, builds it as a 3D
scene a browser can show, and judges any furniture layout against clearance rules: room to walk,
to reach, to open a door, to let light past.

No room, no dimension and no photo lives in this directory. The model is hand-measured data about
somebody's home, so it sits in the active private overlay under `data/wohnung/` and is resolved at
runtime through `libs/overlay`. A test asserts that this is true: no dimension from a real model
appears as a literal anywhere in `src/`.

## Use

```bash
cd capabilities/interior
./interior model                    # the room, and what in it is still a guess
./interior layouts                  # what's on disk
./interior check <layout>           # judge a layout; exits 1 on a hard violation
./interior build                    # build the scene headlessly
./interior open --layout <layout>   # build it and print a browser URL
./interior export --layout <layout> # scene JSON for a render step
```

`check` is a gate: it exits non-zero when a hard rule is broken, so it can sit in front of
anything that would otherwise act on a bad layout.

`INTERIOR_MODEL_DIR` overrides model resolution entirely. That is how the tests run with no
overlay present, and how a second flat gets planned without moving anything.

## What it is built on

The geometry engine is [Pascal](https://github.com/pascalorg/editor), MIT, driven through its MCP
server at a pinned `@pascal-app/mcp@0.3.2` and recorded in `upstreams.toml`. It was adopted rather
than rebuilt: fed a polygon measured on site with a tape it returned the same area to within
0,003 m², and it already knows walls, openings, levels and a browser view.

What it does not know is anybody's rules. Its `check_collisions` is an axis-aligned bounding-box
test between item footprints, which answers "do two things overlap". Every rule in a
`constraints.yaml` is about the space that stays empty, and none of them can be written as an
overlap test. That part is `src/clearance.ts`, and it is the only part of this that is ours.

## How the checker works

Two primitives carry it, both in `src/geometry.ts`.

An exact Euclidean distance transform gives every free point its distance to the nearest
obstacle. Exact, not a chamfer approximation, because 74 cm and 76 cm are opposite verdicts
against a 75 cm floor.

A widest-path search then finds the route between two places that maximises its own narrowest
point. A corridor of width W has clearance W/2 down its centre, so doubling that bottleneck gives
the width a person actually gets.

Access rules ask for a contiguous run rather than a clear strip. A desk at the head end of a bed
blocks part of that side without making the bed unreachable; what decides access is whether there
is a continuous stretch long enough to stand in, taken as 60 cm.

## Things that will bite

**Door-swing zones block furniture, not people.** They constrain where things may stand. Treating
them as walls in the walkability grid made the entrance arc the bottleneck of every route,
reporting an identical number that did not move when furniture did.

**`von`/`bis` on an opening are absolute coordinates along the wall's axis**, not offsets from its
start. The two coincide for any wall starting at 0, which is most of them, so a wall that does not
is where this bites.

**Pascal's HTTP transport accepts one `initialize` per process** and answers later ones with
-32600. This uses stdio, one server per command, which sidesteps it.

**`export_glb` does not work headlessly.** It needs the Three.js renderer, which is browser-only.
`export_json` carries the same geometry; a GLB comes out of the editor by hand.

**Pascal's catalogue is generic presets.** Its `double-bed` is 200 × 250 against a real 160 × 200,
so every item is scaled to its true footprint. Anything with no measured size is not drawn at all
rather than drawn at the preset's, because a plan that is dimensionally a lie is worse than a gap.

## Rules the engine knows

Severity follows the model's own `constraints.yaml`: `hart` blocks, `weich` warns.

| Rule | What it protects |
|---|---|
| walkways | every route between openings stays above the stated minimum |
| approach zones | the clear run in front of a door or a fixed run of units |
| door swings | furniture out of the arc |
| light corridor | nothing tall in the strip nearest the only glazed wall |
| bed access | a contiguous standing run on each long side |
| chair zone | room to push a desk chair back |
| wardrobe doors | room to open them |
| chair pull-out | a dining table nobody can sit at is a shelf; the seat count is reported |
| coffee table gap | far enough from the sofa to get past your own knees |

The last two were in the constraints schema from the beginning and unread by any code path until
2026-08-20, so a layout containing a table passed rules that never ran.
