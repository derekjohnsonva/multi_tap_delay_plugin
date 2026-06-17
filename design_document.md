# Multi-tap Delay — Design Document

_A living spec. Status: drafting. Sections marked **OPEN** need a decision before they're locked._

---

## 1. Concept

A delay effect where the user controls the amplitude **and pan** of each individual discrete echo (tap), via curves and shapes drawn on a graph. Unlike a classic feedback delay (which can only decay), taps here can rise, fall, modulate, or hold — because the topology is a finite multi-tap structure, not a feedback loop.

**The load-bearing architectural fact:** this is a multi-tap delay = an FIR filter. One delay buffer, N read taps at different times, each scaled and panned, summed. No feedback ⇒ unconditionally stable ⇒ arbitrary per-tap gains are safe. No feedback tail in scope.

---

## 2. Decisions locked

| Topic                        | Decision                                                                                                                |
| ---------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| Topology                     | Pure multi-tap (FIR). No feedback tail.                                                                                 |
| Amplitude range              | `0..1` by default. Negative (polarity flip) behind an **Advanced/Polarity** toggle. Nothing above 1 for now.            |
| Gain smoothing               | Yes — smooth per-tap gain/pan changes to kill zipper noise.                                                             |
| Tap-count change             | Preserve existing taps by index; append/remove at the **end**. Stored values persist so toggling N back restores edits. |
| Short delays → comb/spectral | Allowed, not constrained. It's a feature; just signpost it in the UI.                                                   |
| Format                       | CLAP + VST3 plugin in a DAW (CLAP-first). No AU.                                                                        |
| Framework                    | nih-plug (Rust).                                                                                                        |
| Channels                     | Stereo. Per-tap **pan**. Auto **ping-pong**.                                                                            |
| Per-tap params               | Amplitude + pan only. **No** per-tap filter or pitch.                                                                   |
| Tap timing                   | Highly controllable. Both **ms** and **note divisions**.                                                                |
| Curve vs taps                | Curve defines the shape; taps are discrete and sample it. Both editable; individual taps can be broken off the curve.   |
| Curve sources                | Preset shapes (sine, saw, exp, etc.).                                                                                   |
| X-axis                       | Linear time. Labels switch ms ↔ note-value by mode.                                                                     |
| Goal                         | Learning project, but demo-quality look & feel for online showcase.                                                     |

---

## 3. Core architecture: parameter lanes

Everything per-tap is modeled as a **lane**. A lane = a continuous **curve** + **N discrete taps** that sample it + **per-tap detach overrides**.

- **Lanes (now):** Amplitude, Pan.
- **Lane (future, out of scope):** Time-spacing — see §9.
- **Tap value state:** each tap in each lane is either
  - **linked** → value = `curve(x_tap)`, follows the curve live, or
  - **detached** → value = a stored override, ignores the curve.
- **Generators** write a lane: preset shapes write Amplitude; the **ping-pong** generator writes Pan (alternating ±width, optional widening). "Auto ping-pong amount" is just the width scalar of that generator.
- **Editing the curve** moves all _linked_ taps at once. **Dragging a tap** detaches it (or an explicit detach/relink action).

This one model delivers: draw-a-shape editing, per-tap tweaking, preset shapes, ping-pong, and the future time-modulation feature — without special cases.

### Tap-count change rule (codifies the locked decision)

- Taps are indexed `0..N-1`. Increasing N **appends** new taps that start _linked_ (sampled from the current curve). Decreasing N **removes from the end** but retains each removed tap's stored state, so re-increasing restores prior edits.
- Smooth/crossfade the gain of any tap that switches in or out to avoid clicks.

---

## 4. Signal flow

```
input → [write to circular delay buffer]
             │
             ├─ tap 0:  read @ t0  → ×gain0(smoothed) → pan0 → ┐
             ├─ tap 1:  read @ t1  → ×gain1(smoothed) → pan1 → ┤
             ├─ ...                                            ├─ sum (L) ─┐
             └─ tap N:  read @ tN  → ×gainN(smoothed) → panN → ┘ sum (R) ─┤
                                                                           │
input (dry) ───────────────────────────────────────────── ×(1-mix) ──────┤
                                                                          sum → ×outputTrim → [safety limiter?] → output
   wet taps ──────────────────────────────────────────── ×mix ────────────┘
```

- **Per-tap read** with fractional-position interpolation (taps rarely land on sample boundaries when tempo-synced).
- **Pan** uses an equal-power law (−3 dB center) unless decided otherwise (§8).
- **Gain & pan smoothing**: one-pole (~5–50 ms) or per-block ramp on every per-tap coefficient.
- **Level safety**: output meter always visible; output trim; optional safety limiter on the wet path. Summed taps exceed 0 dBFS easily even with each tap ≤ 1.

---

## 5. DSP notes & gotchas

- **CPU is a non-issue at this scale.** Multi-tap is O(N) reads/sample. 128 taps × stereo × interpolation ≈ ~100 M ops/s worst case — trivial for a modern CPU. No FFT/partitioned convolution needed until thousands of taps.
- **Fractional taps**: start with linear interpolation (cheap, fine since we only modulate _amplitude_, not time → no Doppler/pitch artifacts). Upgrade to allpass/sinc later if quality demands.
- **Short total delay (< ~20–50 ms)**: taps comb-filter; the effect becomes a resonator/flanger and the amplitude curve becomes a _spectral_ shaper. Same UI, different behavior — signpost it (e.g. a subtle "comb zone" marker on the time axis), don't block it.
- **Negative amplitude** (Advanced): flips tap polarity; comb peaks ↔ notches. Visual: stems can extend below the amplitude baseline.
- **Zipper noise** sources to smooth: dragging the curve, changing N, changing tap times, toggling ping-pong amount.
- **Tempo**: sync mode needs host BPM (e.g. JUCE `AudioPlayHead`). Standalone / web demo needs a manual BPM field. X-axis labels switch ms ↔ division by mode.

---

## 6. Technology

**Decision: nih-plug (Rust)** — chosen to avoid C++ tooling. Fits the target environment (Linux + Reaper/Bitwig, both of which load CLAP and VST3 natively).

- **DSP** lives in a **separate plain-Rust crate** with no nih-plug dependency, so it compiles independently to WASM for the web demo.
- **Plugin wrapper** via nih-plug; **GUI** in a native Rust framework — **egui** for fast prototyping of the curve editor, or **VIZIA** for a more styled/retained result for the showcase. (No React: nih-plug has no blessed WebView, and React only paid off via JUCE 8's WebView.)
- **Formats:** **CLAP-first** (unencumbered, MIT), VST3 second. **No AU** — unsupported by nih-plug, so no Logic/GarageBand; fine given target DAWs. Optional **standalone** (JACK/CoreAudio/CPAL) build for demoing without a host.

**Licensing caveat:** the framework is ISC, but the VST3 export bindings are **GPLv3** (copyleft) — a VST3 build inherits that. CLAP has no such restriction. Only matters for a closed-source release; ship CLAP for any closed-source path. Non-issue for an open learning project.

**Web demo (shared-core trick, cleaner in Rust):** the DSP crate compiles to **WASM** (wasm-bindgen), driven by an **AudioWorklet** — one DSP core, two targets. The GUI does _not_ carry over; rebuild the editor in web tech, or compile an egui UI to its WASM target.

**Editor rendering:** native GUI is custom-drawn in egui/VIZIA (you're hand-drawing the lanes anyway). For the web demo, Canvas 2D handles ~100 taps + 60 fps metering; SVG can jank under constant redraw.

---

## 7. UI design

**Layout:** a toolbar over two **stacked lanes** sharing one linear time axis; an output meter at the right edge.

- **Amplitude lane** (top): y = `0..1`, stems (lollipops) from baseline, curve overlay, linked vs detached taps visually distinct. (Bipolar when Polarity is on.)
- **Pan lane** (bottom): center = 0, stems up = R / down = L, range `−1..+1`. Ping-pong renders as the alternating pattern.
- **Shared x-axis**: tick labels in note divisions (sync mode) or ms (free mode); a "comb zone" hint at very short times.
- **Meter**: vertical, with a headroom/clip (amber) zone near the top — always visible.

**Toolbar controls (first pass):** tap count · time mode (sync division / free ms) + length · smoothing · mix · ping-pong amount · output trim · polarity (advanced) · per-lane preset-shape picker.

**Interactions:**

- Drag the **curve** → moves all linked taps in that lane.
- Drag a **tap** → detaches it (override); right-click / modifier to relink.
- Pick a **preset shape** → writes the lane (linked taps re-sample; detached taps keep their overrides unless "reset all").
- Scale tap count → append/remove at end per §3.

**Scaling with N:** below ~32 taps, per-tap dragging is primary. Above ~32, the curve becomes the primary tool and per-tap detaching is the exception. Suggested default N ≈ 8–16, soft max 128.

---

## 8. Future / out of scope

- **Time modulation** — tap spacing accelerates/decelerates over time (ritardando/accelerando). Natural fit: a third **Time-spacing lane** in the same model (a spacing curve the taps sample). Explicitly deferred.
- Amplitude boost above 1 (with compensation).
- Per-tap filter / pitch (Delay-Designer-style), if ever.
- Curve types beyond presets (bezier handles, freehand).
