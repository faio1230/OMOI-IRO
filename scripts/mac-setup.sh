#!/usr/bin/env bash
# OMOI-IRO: build and check on macOS (Apple Silicon) in one go. No Link.
#   bash scripts/mac-setup.sh            # from the repository root
# Steps: tools -> cargo test -> release probe -> port list -> desktop app
# (npm ci + tauri build; falls back to --no-bundle) -> where things are.
# Sends nothing to MIDI ports: `omoiiro-probe ports` only lists them.
set -euo pipefail

cd "$(dirname "$0")/.."
REPO="$(pwd)"
TARGET="${CARGO_TARGET_DIR:-$REPO/target}"

step() { printf '\n== %s\n' "$*"; }

step "tools"
xcode-select -p
uname -m
sw_vers -productVersion
cargo --version
rustc --version
node --version
npm --version

step "cargo test --workspace"
cargo test --workspace

step "cargo build --release -p omoiiro-probe"
cargo build --release -p omoiiro-probe

step "MIDI ports (copy the exact names into the terminal settings)"
"$TARGET/release/omoiiro-probe" ports

step "desktop app"
cd "$REPO/apps/desktop"
npm ci
BUNDLED=1
if ! npx tauri build; then
  echo "tauri build (bundle) failed; building the bare binary with --no-bundle"
  BUNDLED=0
  npx tauri build --no-bundle
fi
cd "$REPO"

step "results"
echo "probe:   $TARGET/release/omoiiro-probe"
echo "app bin: $TARGET/release/omoiiro"
if [ "$BUNDLED" = 1 ]; then
  ls -d "$TARGET"/release/bundle/macos/*.app 2>/dev/null || true
  ls "$TARGET"/release/bundle/dmg/*.dmg 2>/dev/null || true
fi
echo "terminal settings: $HOME/Library/Application Support/jp.faio.omoiiro/terminal-settings.json"
echo "example:           $REPO/fixtures/terminal-settings.mac.example.json"
