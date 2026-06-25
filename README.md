# delay_plugin

A multi-tap delay audio effect, built in Rust with [nih-plug](https://github.com/robbert-vdh/nih-plug) and exported as **CLAP** and **VST3**.

The workspace is split into two crates:

- **`delay-core`** — the DSP, with no plugin/host dependencies (pure Rust, unit-testable in isolation).
- **`delay-plugin`** — the nih-plug wrapper (parameters, editor, CLAP/VST3 export) around `delay-core`.

See `design_document.md` for the spec and `PLAN.md` for the implementation plan.

## Prerequisites

- A recent stable **Rust** toolchain (`rustup`, `cargo`). No `rust-toolchain.toml` is pinned, so the default stable works.
- For installing into a DAW that ships its own runtime (e.g. the Bitwig Flatpak), **cargo-zigbuild** + **Zig**:
  ```bash
  cargo install cargo-zigbuild
  # and install zig (e.g. `yay -S zig` or from https://ziglang.org/download)
  ```
- For validation (optional): `clap-validator` (`yay -S clap-validator`) and `pluginval`.

## Build & test

Standard cargo, run from the repo root:

```bash
cargo build            # debug build of all crates
cargo test             # run the test suite (most DSP tests live in delay-core)
cargo clippy --all-targets
cargo fmt
```

## Bundling the plugin (CLAP / VST3)

The `xtask` crate runs nih-plug's bundler. `cargo xtask` is aliased in `.cargo/config.toml`, so:

```bash
cargo xtask bundle delay-plugin --release
```

This writes the bundles to `target/bundled/`:

- `target/bundled/delay-plugin.clap`
- `target/bundled/delay-plugin.vst3`

These are linked against the **host** glibc. They're correct for `cargo test`, the validators, and any DAW running on the host system — but **not** for a DAW sandbox with an older glibc (see below).

## Installing into Bitwig (Flatpak)

> Bitwig here runs as a **Flatpak** (freedesktop 22.08 runtime = glibc 2.35). The plain `cargo xtask bundle` output links against the Arch host glibc (much newer) and fails to load inside the sandbox with `GLIBC_… not found`.

Use the helper script, which cross-links against glibc 2.35 with cargo-zigbuild and installs real files (not symlinks) into the paths Bitwig scans:

```bash
scripts/bundle-flatpak.sh
```

It produces and installs:

- `~/.clap/delay-plugin.clap`
- `~/.vst3/delay-plugin.vst3/Contents/x86_64-linux/delay-plugin.so`

It also prints the highest required `GLIBC_*` symbol version (should be ≤ 2.35). After it finishes, **rescan plugins in Bitwig** to pick up the new build.

Do **not** symlink the `target/bundled/` artifacts into `~/.clap` / `~/.vst3` — that's the host-glibc build and won't load in the Flatpak.

## Validating the plugin

Both validators run natively on the host, so they validate the `target/bundled/` (host-glibc) output.

**CLAP** — wrapper script that builds then validates:

```bash
scripts/validate-clap.sh             # build (release) then validate
scripts/validate-clap.sh --no-build  # validate the existing bundle
scripts/validate-clap.sh --debug     # build the debug profile instead
```

Its exit code is the validator's (0 = all passed), so it's safe to gate CI or a pre-push hook on.

**VST3** — with `pluginval`:

```bash
pluginval --strictness-level 10 --validate target/bundled/delay-plugin.vst3
```

## Manual testing

`MANUAL_TEST_CHECKLIST.md` has the in-DAW checklist for behaviour the automated validators don't cover.

## Common tasks at a glance

| Task | Command |
| --- | --- |
| Build everything (debug) | `cargo build` |
| Run tests | `cargo test` |
| Lint / format | `cargo clippy --all-targets` / `cargo fmt` |
| Bundle CLAP + VST3 (host) | `cargo xtask bundle delay-plugin --release` |
| Build & install for Bitwig Flatpak | `scripts/bundle-flatpak.sh` |
| Validate CLAP | `scripts/validate-clap.sh` |
| Validate VST3 | `pluginval --strictness-level 10 --validate target/bundled/delay-plugin.vst3` |
