#!/usr/bin/env bash
# Run clap-validator against the built CLAP bundle as part of the test pipeline.
#
# Unlike the Bitwig Flatpak (which needs the glibc-2.35 zigbuild, see
# bundle-flatpak.sh), clap-validator runs natively on this host, so the plain
# `cargo xtask bundle` output — linked against the host glibc — loads fine. We
# therefore build/validate the standard bundle, not the Flatpak one.
#
# Install the validator first (stable AUR release):  yay -S clap-validator
#
# Usage:
#   scripts/validate-clap.sh            # build (release) then validate
#   scripts/validate-clap.sh --no-build # validate the existing bundle
#   scripts/validate-clap.sh --debug    # build the debug profile instead
#
# Exit code is the validator's: 0 = all tests passed, non-zero = a test failed
# (or errored), so this is safe to gate CI / a pre-push hook on.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

BUILD=1
PROFILE="release"
for arg in "$@"; do
  case "$arg" in
    --no-build) BUILD=0 ;;
    --debug) PROFILE="debug" ;;
    --release) PROFILE="release" ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

if ! command -v clap-validator >/dev/null 2>&1; then
  echo "error: clap-validator not found on PATH." >&2
  echo "       install it with:  yay -S clap-validator" >&2
  exit 127
fi

if [[ "$BUILD" == 1 ]]; then
  echo ">> building CLAP bundle (${PROFILE})"
  if [[ "$PROFILE" == "release" ]]; then
    cargo xtask bundle delay-plugin --release
  else
    cargo xtask bundle delay-plugin
  fi
fi

CLAP="target/bundled/delay-plugin.clap"
if [[ ! -e "$CLAP" ]]; then
  echo "error: $CLAP not found — run without --no-build first." >&2
  exit 1
fi

echo ">> clap-validator $(clap-validator --version 2>/dev/null || echo '?')"
echo ">> validating $CLAP"
# Each test runs out-of-process by default so a crashing test can't take the
# whole run down. Let the validator's exit code propagate to the caller.
clap-validator validate "$CLAP"
