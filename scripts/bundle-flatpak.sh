#!/usr/bin/env bash
# Build the CLAP plugin against an OLD glibc so it loads inside DAWs that ship
# their own runtime (e.g. the Bitwig Flatpak on freedesktop 22.08 = glibc 2.35),
# even though this Arch host has a much newer glibc.
#
# We use cargo-zigbuild, which uses Zig as the cross-linker and lets us pin the
# target's glibc version via the `.2.35` suffix. A plain `cargo xtask bundle`
# links against the host glibc and fails to load in the older sandbox.
#
# Same zigbuilt .so feeds both formats — the glibc fix is format-agnostic. The
# only difference is packaging: a .clap is the cdylib renamed, while a VST3 on
# Linux is a bundle directory (Contents/x86_64-linux/<name>.so). We assemble
# both by hand because `cargo xtask bundle` links against the host glibc.
#
# Usage: scripts/bundle-flatpak.sh   (run from the repo root or anywhere)
set -euo pipefail

# Oldest glibc we need to support. The Bitwig Flatpak runs on freedesktop 22.08
# (glibc 2.35); bump this only if a target host needs newer.
GLIBC="2.35"
TARGET="x86_64-unknown-linux-gnu.${GLIBC}"
TARGET_DIR="x86_64-unknown-linux-gnu" # cargo strips the glibc suffix here

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

echo ">> cargo zigbuild --release (target ${TARGET})"
cargo zigbuild --release -p delay-plugin --target "${TARGET}"

SO="target/${TARGET_DIR}/release/libdelay_plugin.so"

# --- CLAP: just the cdylib renamed ----------------------------------------
CLAP_DEST="${HOME}/.clap/delay-plugin.clap"
mkdir -p "${HOME}/.clap"
# Replace the dev symlink (which pointed at the host-glibc xtask bundle) with the
# zigbuilt artifact so the path Bitwig scans always holds the loadable build.
rm -f "${CLAP_DEST}"
cp "${SO}" "${CLAP_DEST}"
echo ">> installed CLAP: ${CLAP_DEST}"

# --- VST3: bundle directory layout ----------------------------------------
VST3_BUNDLE="${HOME}/.vst3/delay-plugin.vst3"
VST3_SO_DIR="${VST3_BUNDLE}/Contents/x86_64-linux"
# Drop any prior symlink/bundle, then build the standard layout fresh.
rm -rf "${VST3_BUNDLE}"
mkdir -p "${VST3_SO_DIR}"
cp "${SO}" "${VST3_SO_DIR}/delay-plugin.so"
echo ">> installed VST3: ${VST3_BUNDLE}"

echo ">> verifying max required GLIBC symbol version:"
# Should print nothing above ${GLIBC}; anything higher means the pin didn't take.
objdump -T "${SO}" 2>/dev/null \
  | grep -oE 'GLIBC_[0-9]+\.[0-9]+' | sort -uV | tail -3 || true
echo ">> done. Rescan plugins in Bitwig to pick up both formats."
