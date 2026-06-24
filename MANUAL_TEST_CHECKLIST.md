# Manual test & stress checklist

Hands-on verification for things automated checks **can't** catch — audible
artifacts, GUI behaviour, host integration, and DSP under extreme settings. Work
top-down; the early sections expose the most.

**Already covered automatically** (don't re-verify by hand — all green in
`clap-validator`, see `scripts/validate-clap.sh`): parameter NaN/range handling,
state reproducibility, out-of-place processing, subnormal output, basic GUI
create/destroy. VST3 additionally passes `pluginval` at strictness 10.

---

## 1. Audio engine under load (the multi-tap core)
- [ ] **Max taps, short spacing.** 128 taps + Free time ~5 ms (deep comb zone). Should be a resonant/flanged tone — **no NaN dropout, no runaway level**. Watch the meter.
- [ ] **Max taps, max spacing.** 128 taps in Free mode with the last tap near the 10 s buffer limit. Late taps must **clamp** cleanly (no garbage/wraparound noise).
- [ ] **Tap-count sweep while playing.** Drag Taps 1→128→1 rapidly on sustained audio. Must **fade, not click**.
- [ ] **Single tap (N=1)** edge case: one clean echo, panned correctly, no divide-by-zero in x-position math.
- [ ] **Summed-gain overload.** Flat amp shape + 128 taps + Mix 100% → wet sum far exceeds 0 dBFS (meter amber/red). Toggle the **Limiter**: output bounds without ugly pumping, and is transparent when the signal is low.

## 2. Smoothing / zipper (set Smoothing = 0 ms to stress)
- [ ] With **Smoothing at 0**, rapidly automate **Mix**, **Output**, **Amp Amount**, **Ping-Pong**. Listen for zipper/steps; raise smoothing and confirm it cleans up.
- [ ] **Polarity flip while playing** sustained audio — must not hard-click.
- [ ] **Drag a tap fast** in the GUI on sustained audio — gain/pan should glide, not jump.

## 3. Lane interactions (the detach/relink model — the design spine)
- [ ] **Retention rule:** detach + drag taps 5–8, set Taps down to 3, then back to 8. Earlier edits must **return exactly** (high-water-mark retention).
- [ ] **Source vs detached:** detach a couple taps, then change Amp Shape / drag the curve. **Linked taps move, detached stay put.**
- [ ] **Relink paths:** double-click and right-click a detached tap → relinks. **Reset** relinks all.
- [ ] **Extremes:** drag taps to the very top/bottom (and pan hard L/R). Values clamp; markers render correctly.
- [ ] **Bipolar amp:** enable Polarity, drag amp taps below the centerline → audible polarity inversion; baseline renders at center.

## 4. State persistence (real host, with the GUI)
- [ ] **Full round-trip:** set unusual params, detach/edit several taps in both lanes, **save project → close → reopen.** Everything restored: params, detached overrides, tap count, **editor window size**.
- [ ] **Preset across instances:** save a preset, load it on a *fresh* instance → identical.
- [ ] **Save-without-playback:** load a project but don't start transport, then open the editor — lanes render correctly, not defaults. *(The flush-path bug class.)*
- [ ] **Shrunk-and-saved:** detach tap 15, set Taps to 4, save, reload, grow back to 16 → edit at 15 restored.

## 5. Tempo sync
- [ ] **Live BPM change** while playing → echo timing tracks immediately.
- [ ] **Extreme BPM** (e.g. 20 and 300) → spacing stays sane; **triplet & dotted** divisions time correctly against a metronome.
- [ ] **Transport stop/start** and **tempo automation/ramp** → no glitches; free-running fallback (120 BPM) when the host reports no tempo.

## 6. Host / format integration
- [ ] Load **both CLAP and VST3** in Bitwig; if possible a second host (Reaper/Ardour) — same behaviour.
- [ ] **Sample-rate changes** (44.1 / 48 / 96 k): reinitialize cleanly; delay times stay correct in ms and in sync.
- [ ] **Buffer-size changes** (32 vs 2048): no artifacts; smoothing sounds the same regardless of block size.
- [ ] **Offline bounce vs realtime:** render the same passage both ways → audibly/measurably identical.
- [ ] **Bypass**, **multiple instances**, **open/close editor repeatedly** → no crash, no leak, no stuck audio.

## 7. CPU & the audio/GUI boundary
- [ ] 128 taps × several instances → check CPU headroom.
- [ ] **GUI open vs closed** shouldn't change audio behaviour — and a busy GUI must never stall audio (the `try_write` design). Stress by dragging in the editor while watching for audio dropouts/xruns.

## 8. Degenerate / robustness inputs
- [ ] **Pure silence in → silence out** (no self-noise, no denormal "crackle" as exp-decay taps approach zero).
- [ ] **Hot/clipping input** → limiter behaviour sane.
- [ ] **Everything automated at once** (DAW automation on all params simultaneously) → no crash, no NaN.
- [ ] **Mono track / mono-sum check:** ping-pong at high tap count summed to mono — no severe phase-cancellation surprises.
