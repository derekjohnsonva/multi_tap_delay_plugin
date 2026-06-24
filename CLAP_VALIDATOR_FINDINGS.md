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

## Result: 21 tests — **16 passed, 5 skipped, 0 failed** ✅

- **5 skipped** are all note/MIDI-port tests — expected, we're an audio effect
  with no note ports.
- The `state-reproducibility-flush` failure described below has been **fixed**
  (see "The fix"); all three `state-reproducibility-*` tests now pass.

## The bug that was found (and fixed)

Originally `state-reproducibility-flush` **failed** — a real plugin bug, not a
validator quirk.

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

### The fix

`Lane` now persists **only its per-tap detach overrides** (a sparse, ascending
`Vec<(index, value)>`), via a `#[serde(into/from = "LanePersist")]` proxy. The
`source`, clamp `range`, and `active` count are **derived from the params** —
which persist separately — and are reconstructed at runtime by
`DelayParams::apply_to_lanes`, called from **both**:

- the audio thread (`process` → `update_taps`, non-blocking `try_write`), and
- the editor (a brief blocking `write` before it renders),

so neither path depends on the other having run. With no derived/process-only
state in the serialized form, the flush-path and process-path saves are
byte-identical for identical params, and `state-reproducibility-flush` passes.

Two `delay-core` unit tests pin this: `serde_round_trip_preserves_detach_overrides`
(overrides survive; derived fields don't) and `serialization_ignores_derived_fields`
(two lanes with different source/range/count but no overrides serialize
identically — the exact regression).
