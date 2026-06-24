# Multi-tap Delay — Implementation Plan

_PR-sized breakdown of the work described in [design_document.md](./design_document.md). Each PR is independently reviewable, leaves the project in a building/loadable state, and is ordered so later PRs build on earlier ones._

> **Progress:** Phases 0–3 (PR 1–13) are ✅ **done** — workspace scaffolded; `delay-core` engine + lane/curve model; the plugin now exposes the full param set, persists lane state, and wires params → engine with tempo sync, so the CLAP + VST3 bundle is an audible multi-tap delay. **Phase 4 (egui editor) ✅ done:** PR 14 ✅ — `nih_plug_egui` editor scaffold + toolbar (all global params bound via `ParamSlider`; persisted window size). PR 15 ✅ — custom-drawn **amplitude lane**: per-tap stems/lollipops from a baseline with the source curve traced behind them, linked vs. detached taps drawn distinctly, bipolar layout when polarity is on. Added `LaneSource::value_at(x)` for continuous curve sampling. PR 16 ✅ — **pan lane** stacked below: always bipolar (centre = 0, up = R / down = L), with ping-pong rendered as the alternating zig-zag connecting the tap tips; the amp-lane renderer was generalised into one `draw_lane` serving both lanes. PR 17 ✅ — **shared time axis** below both lanes with tick labels that switch ms (free mode) ↔ division-multiples (sync mode), plus a subtle **comb-zone** shade over the short-time region of both lanes and the axis; the audio thread publishes the current BPM to the editor via an `Arc<AtomicF32>` so sync-mode tap times can be located in ms. PR 18 ✅ — **output meter**: the engine tracks a decaying post-trim peak (`output_level()`), published to the editor via an `Arc<AtomicF32>` and drawn as an always-visible vertical dB meter pinned to the right edge, with an amber headroom/clip zone above 0 dBFS and a red fill when clipping. PR 19 ✅ — **lane interactions**: drag a tap to detach + set its value, drag the background curve to nudge the source amount (moving all linked taps via the param's gesture path), and right-click a tap to relink it; the lane renderer was split into `paint_lane` (read snapshot) + `handle_lane_input` (brief write lock), sharing a `LaneGeom` for hit-testing, with per-gesture drag state in egui temp storage. **Phase 4 (egui editor, PR 14–19) complete.** **Phase 5 (polish & release-readiness) ✅ done (bar pluginval):** PR 20 ✅ — optional `delay-core::Limiter` safety limiter on the summed wet path (instantaneous attack to a ≈ −0.1 dBFS ceiling, ~100 ms release; toolbar checkbox, off by default). PR 21 ✅ — cohesive dark-theme styling pass (`apply_theme` + palette), and a **state-reproducibility fix** caught by `clap-validator` (`Lane` now persists only the per-tap detach overrides via custom serde, since count/source/range are re-derived from params each block). 69 unit tests pass workspace-wide; clippy clean; both bundles build with the GUI; **`clap-validator validate` is clean (16 passed / 0 failed / 5 N/A skips)**. **All 21 planned PRs are now implemented.** **Not yet verified by a human:** loading in a DAW (Reaper/Bitwig) for listening + visual QA of the editor/styling; `pluginval` (VST3) is not installed (external JUCE binary — authorize or run manually; the state fix lives in shared `Params` serialization so it applies to VST3 too).

---

## Context

We are building a multi-tap (FIR) delay plugin in Rust with [nih-plug](https://github.com/robbert-vdh/nih-plug), targeting **CLAP-first + VST3**. The defining feature is per-tap **amplitude and pan** control via drawable curves/lanes (see design doc §1–§3). There is no feedback, so the engine is unconditionally stable and per-tap gains are arbitrary.

**Scope of this plan:** the plugin (CLAP + VST3) end-to-end with a native **egui** editor. The WASM web demo and standalone build are explicitly *out of scope here* and noted as future phases (design doc §6, §8).

**Two key architectural commitments that shape every PR below:**

1. **DSP lives in a separate plain-Rust crate** (`delay-core`) with **no nih-plug dependency**, so it stays unit-testable without a host and can compile to WASM later. The plugin crate (`delay-plugin`) is a thin wrapper + egui GUI.
2. **The "lane" model is the spine** (design doc §3): a lane = a continuous curve + N discrete taps that sample it + per-tap detach overrides. Amplitude and Pan are both lanes. Build the engine first, then the lane model on top, then the params, then the GUI.

---

## Repository structure (target)

```
delay_plugin/
├─ Cargo.toml              # workspace
├─ crates/
│  ├─ delay-core/          # plain Rust, no nih-plug — all DSP + lane model
│  │  └─ src/
│  │     ├─ lib.rs
│  │     ├─ buffer.rs      # circular delay buffer + fractional read
│  │     ├─ engine.rs      # multi-tap engine (reads, gain, pan, sum, mix)
│  │     ├─ lane.rs        # curve + taps + linked/detached model
│  │     ├─ curves.rs      # preset shapes + ping-pong generator
│  │     └─ smoothing.rs   # one-pole / ramp smoothers
│  └─ delay-plugin/        # nih-plug wrapper + egui editor
│     └─ src/
│        ├─ lib.rs         # Plugin impl, Params, process()
│        ├─ params.rs      # nih-plug Params struct
│        └─ editor/        # egui editor: toolbar, lanes, meter, interactions
└─ design_document.md
```

---

## Phase 0 — Scaffolding

### PR 1 — Workspace + loadable passthrough plugin
- Create the Cargo workspace and both crates (`delay-core` empty stub, `delay-plugin` depending on it).
- Add `nih_plug` + `nih_plug_egui` to `delay-plugin`; implement the minimal `Plugin` + `ClapPlugin` + `Vst3Plugin` traits as a **stereo dry passthrough** (no delay yet).
- Add the `nih_plug` xtask bundler (`cargo xtask bundle delay-plugin --release`) so CLAP/VST3 artifacts build.
- **Verify:** bundle builds; plugin loads in Reaper/Bitwig and passes audio through unchanged; `clap-validator validate` passes.

---

## Phase 1 — Core DSP engine (`delay-core`, host-free, unit-tested)

### PR 2 — Circular delay buffer with fractional read
- `buffer.rs`: a per-channel circular buffer; `write(sample)` and `read(delay_samples: f32)` with **linear interpolation** (design doc §5).
- **Verify:** unit tests — integer delays return exact samples; fractional delays interpolate; wrap-around correctness; reading beyond filled length returns silence.

### PR 3 — Single tap → multi-tap engine (mono)
- `engine.rs`: `Tap { delay_samples, gain }`; engine holds the buffer + `Vec<Tap>`, sums all tap reads per sample. Mono path first to keep the math obvious.
- **Verify:** unit tests — one tap at delay D with gain g reproduces a scaled, delayed impulse; N taps sum correctly; impulse response matches expected FIR.

### PR 4 — Stereo + per-tap pan (equal-power)
- Extend `Tap` with `pan: f32` (−1..+1); equal-power pan law (−3 dB center, design doc §4/§8). Engine outputs stereo (L/R sums).
- **Verify:** unit tests — pan = 0 → equal L/R at −3 dB; pan = ±1 → fully one side; power sums to unity across the sweep.

### PR 5 — Per-coefficient smoothing
- `smoothing.rs`: one-pole smoother (~5–50 ms) applied to each tap's gain and pan so changes don't zipper (design doc §4/§5). Engine reads smoothed coefficients per sample.
- **Verify:** unit tests — step change in gain ramps over the configured time constant rather than jumping; no discontinuity at block boundaries.

### PR 6 — Dry/wet mix + output trim
- Engine-level `mix` (dry vs summed wet) and `output_trim` gain stage (design doc §4 signal flow).
- **Verify:** unit tests — mix=0 → dry only; mix=1 → wet only; trim scales output linearly.

---

## Phase 2 — Lane / curve model (`delay-core`)

### PR 7 — Lane abstraction (curve + taps + linked/detached)
- `lane.rs`: a `Lane` owns a curve (sampleable `value(x) -> f32`) and `N` taps, each `Linked` (value = `curve(x_tap)`) or `Detached(override)` (design doc §3). API to set N, sample all linked taps, detach/relink a tap, and read the resolved value for each tap.
- **Verify:** unit tests — linked taps follow the curve; detaching freezes a value; relinking re-samples; editing the curve moves all linked taps and leaves detached ones untouched.

### PR 8 — Preset curve shapes
- `curves.rs`: preset amplitude shapes — sine, saw, exponential decay, etc. (design doc §2/§3). Each is a parametric `value(x)` over the normalized lane domain.
- **Verify:** unit tests — sampled shapes match expected values at known x; selecting a preset re-samples linked taps only.

### PR 9 — Ping-pong pan generator
- `curves.rs`: a generator that **writes the Pan lane** as alternating ±width per tap, with optional widening (design doc §3 — "ping-pong is just the width scalar of that generator").
- **Verify:** unit tests — alternating sign by tap index; width scalar scales magnitude; widening ramps width across taps.

### PR 10 — Tap-count change rule
- Implement the locked rule (design doc §3): increasing N **appends** linked taps sampled from the curve; decreasing N **removes from the end** but **retains** removed taps' stored state so re-increasing restores prior edits. Crossfade/smooth gain of taps switching in/out.
- **Verify:** unit tests — edit tap 5, drop N below 5, raise it back → edit restored; appended taps start linked; no click on count change (gain ramps).

---

## Phase 3 — Plugin wrapper & parameters (`delay-plugin`)

### PR 11 — Params struct
- `params.rs`: nih-plug `Params` for global controls (design doc §7 toolbar) — tap count, time mode (sync division / free ms) + length, smoothing time, mix, ping-pong amount, output trim, polarity (advanced). Use `#[persist]` for the lane state (per-tap detach overrides + stored values).
- **Verify:** params show in the host's generic UI; values persist across save/reload of the host project.

### PR 12 — Wire params → engine + tempo sync
- In `process()`, translate params into engine config each block: compute tap delay times from ms or note-division × host BPM (via nih-plug `Transport`), push gain/pan/mix/trim into the engine. Standalone-less for now; sync uses host tempo.
- **Verify:** in Reaper/Bitwig — audible multi-tap echoes; changing tap count/length/mix behaves; tempo-synced divisions track host BPM changes.

### PR 13 — Polarity (advanced) + comb-zone behavior
- Allow negative amplitude behind the polarity toggle (design doc §2/§5). Confirm short total delays comb/flange naturally (no blocking — it's a feature).
- **Verify:** polarity flip audibly inverts tap polarity; very short lengths produce a resonator/flanger character without artifacts.

---

## Phase 4 — egui editor (`delay-plugin/editor`)

### PR 14 — Editor scaffold + toolbar
- `nih_plug_egui` editor window; toolbar widgets bound to the Params from PR 11 (tap count, time mode + length, smoothing, mix, ping-pong, output trim, polarity, per-lane preset picker). No custom lane drawing yet.
- **Verify:** editor opens in host; toolbar controls move the same params as the generic UI and affect audio.

### PR 15 — Amplitude lane rendering
- Custom-drawn lane: y = 0..1, stems/lollipops from baseline, curve overlay, linked vs detached taps visually distinct (design doc §7). Bipolar when polarity is on.
- **Verify:** stems reflect current tap gains live; preset selection redraws; detached taps render distinctly.

### PR 16 — Pan lane rendering
- Custom-drawn pan lane: center = 0, stem up = R / down = L, range −1..+1; ping-pong renders as the alternating pattern (design doc §7).
- **Verify:** pan values render correctly; ping-pong generator produces the expected visual zig-zag.

### PR 17 — Shared time axis + comb-zone hint
- One linear x-axis shared by both lanes; tick labels switch ms ↔ note-division by mode; subtle "comb zone" marker at very short times (design doc §5/§7).
- **Verify:** labels switch with time mode; comb-zone hint appears at short lengths.

### PR 18 — Output meter
- Vertical meter at the right edge with a headroom/clip (amber) zone near the top, always visible (design doc §4/§7). Reads post-trim wet+dry level from the engine.
- **Verify:** meter tracks output level; amber zone lights as summed taps approach/exceed 0 dBFS.

### PR 19 — Lane interactions
- Drag the **curve** → moves all linked taps in that lane. Drag a **tap** → detaches it (override). Right-click / modifier → relink. Preset pick re-samples linked taps, keeps detached overrides (design doc §7 interactions). Route edits through the PR 7–10 lane API with smoothing.
- **Verify:** dragging curve moves linked taps and audio follows; dragging a tap detaches; relink works; no zipper noise on drags.

---

## Phase 5 — Polish & release-readiness

### PR 20 — Optional safety limiter on wet path ✅
- Optional limiter on the summed wet signal (design doc §4 — summed taps exceed 0 dBFS easily). Toggle/visible in UI.
- **Done:** new `delay-core::Limiter` — a stereo feed-forward peak limiter (instantaneous attack so output never exceeds a ≈ −0.1 dBFS ceiling, ~100 ms release, no lookahead/latency) applied to the summed wet signal before the dry mix; a no-op when disabled. Wired behind a `limiter` `BoolParam` (off by default) with a toolbar checkbox. Tests: limiter caps a clipping multi-tap sum to ≤ 1.0, stays transparent when off, leaves quiet signal untouched, and releases after a peak. 67 tests pass workspace-wide; clippy clean; glibc-2.35 bundle rebuilt + installed.
- **Not yet verified by a human:** listening test with many high-gain taps in the DAW.

### PR 21 — Styling pass + state round-trip + validation ✅ (pluginval pending)
- Showcase-quality styling pass (design doc §1 goal: demo-quality look & feel). Full state save/restore validation; `clap-validator` + `pluginval` (VST3) clean runs.
- **Done:**
  - **Styling:** a cohesive dark theme applied once at editor startup (`apply_theme`) — defined palette (accent / detached / backgrounds / hairline), recessed lane tracks, accented controls, tighter spacing. Lane + meter drawing now reference the palette constants.
  - **State round-trip — found & fixed a real bug:** `clap-validator`'s `state-reproducibility-flush` failed because `Lane` serialized its host-derived fields (source, active count, range), which only sync inside `process()`; an instance that only got `flush()` saved different state. Fixed with custom `serde` for `Lane` that persists **only the per-tap detach overrides** (count/source/range are always re-derived from params each block). Covered by `delay-core` serde tests + a `delay-plugin` `serialize_fields`/`deserialize_fields` round-trip test.
  - **Validation:** `clap-validator validate` is clean — **16 passed, 0 failed, 5 skipped** (skips are note-port checks, N/A for an audio FX).
- **Remaining / not yet verified by a human:** `pluginval` (VST3) is not installed (external JUCE binary download — needs authorization; the state fix lives in shared `Params` serialization so it applies to VST3 too); visual QA of the styling in the DAW; listening QA.

---

## Future phases (out of scope here — see design doc §6, §8)

- **Standalone build** (CPAL/JACK) for host-free demoing.
- **WASM web demo**: compile `delay-core` to WASM (wasm-bindgen) driven by an AudioWorklet; rebuild the editor in web tech (Canvas 2D).
- **Time-spacing lane** (ritardando/accelerando) — a third lane in the same model.
- Amplitude boost above 1; per-tap filter/pitch; bezier/freehand curves.

---

## Verification strategy (overall)

- **`delay-core`**: pure unit tests (`cargo test -p delay-core`) — no host needed. This is where DSP correctness is proven (buffer, taps, pan, smoothing, lane model, tap-count rule).
- **`delay-plugin`**: build with `cargo xtask bundle delay-plugin --release`; manual load + listening tests in **Reaper** and/or **Bitwig** (both load CLAP and VST3 natively per design doc §6).
- **Automated plugin checks**: `clap-validator validate` for CLAP; `pluginval` for VST3, run from PR 1 onward and again at PR 21.
- **Dependency order**: Phases are sequential. Within Phase 1–2, PRs build linearly; Phase 4 PRs depend on Phase 3 params and Phase 2 lane API but are otherwise independent of each other and could be parallelized after PR 14.
