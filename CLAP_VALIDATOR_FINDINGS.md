# clap-validator: setup + findings (experiment branch)

Experiment to get [clap-validator](https://github.com/free-audio/clap-validator)
running and wired into the test pipeline.

## What's here

- **`scripts/validate-clap.sh`** — builds the native CLAP bundle and runs the
  validator locally. Exit code propagates (0 = pass), so it can gate a pre-push
  hook or CI. Native host glibc is fine here — the glibc-2.35 zigbuild
  (`bundle-flatpak.sh`) is only needed for the Bitwig Flatpak sandbox, not for
  the natively-run validator.
- **`.github/workflows/ci.yml`** — `test + clippy` job and a `clap-validator`
  job that installs the validator (`cargo install --git`), bundles, and runs it
  under `xvfb` (for the GUI-creation tests).

## Install

```
yay -S clap-validator              # stable AUR release (0.3.2)
# or, no sudo:
cargo install --git https://github.com/free-audio/clap-validator.git --locked
```

## Current result: 21 tests — 15 passed, 5 skipped, **1 failed**

- **5 skipped** are all note/MIDI-port tests — expected, we're an audio effect
  with no note ports.
- **1 failure: `state-reproducibility-flush`.** This is a **real plugin bug**,
  not a validator quirk.

### Root cause (confirmed by diffing the two state files)

The persisted `amp_lane` / `pan_lane` (`#[persist]`) are mutated as a **side
effect of `process()`** (via `update_taps()` calling `set_source` / `set_range`
/ `set_count`). The validator's flush test saves state after `flush()` — which
does **not** run `process()` — so the lanes are still at their defaults, whereas
the process-path save reflects the params. Same params, different saved state →
fail.

Evidence — `params` blocks are identical; only the persisted `fields` differ:

| field        | flush-path save (expected)        | process-path save (actual)              |
|--------------|-----------------------------------|-----------------------------------------|
| amp_lane.source | `ExpDecay{k:3.0}` (default)    | `Triangle{cycles:0.546}` (from params)  |
| amp_lane.active | `8` (default)                  | `15` (from `taps` param)                |
| pan_lane.width  | `0.5` (default)                | `0.220` (from `pingpong` param)         |

### Why it matters

The lane's `source`, `min`/`max`, and `active` are all **derived from params**
(amp_shape+amp_amount, polarity, tap_count, pingpong_amount). Persisting them is
both redundant and the source of this nondeterminism. The only genuinely
user-authored lane state is the **per-tap detach overrides**.

### Recommended fix (intersects the active lane/GUI work, so left for the owner)

Make `process()` treat the persisted lanes as **read-only**: derive
source/range/count locally and compute per-tap gain/pan without writing back
into the `RwLock<Lane>`. Persist only the detach-override vector; reconstruct the
derived fields from params on load. With no persisted state mutated in
`process()`, the flush and process save paths produce identical state and the
test passes.

(Not implemented on this branch because it touches the lane persistence model
that PR 16–19 actively develop — surfacing it here for integration.)
