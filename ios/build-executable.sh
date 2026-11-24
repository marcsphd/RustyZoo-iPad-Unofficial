#!/bin/bash
set -euo pipefail

project="$(cd "$(dirname "$0")/.." && pwd)"
workspace="$(dirname "$project")"
platform="$1"
destination="$2"

case "$platform" in
    iphoneos) target=aarch64-apple-ios; cache_folder=rustyzoo-ipad-target ;;
    iphonesimulator) target=aarch64-apple-ios-sim; cache_folder=rustyzoo-ipad-sim-target ;;
    *) echo "Expected iphoneos or iphonesimulator" >&2; exit 2 ;;
esac

local_build="$workspace/ipad-local-build"
build_root="$workspace"
rust_dir="$workspace/ipad-toolchain/rust/bin"
cargo_root="$workspace/rustyzoo-cargo"
if [[ -x "$local_build/rust/bin/cargo" && -d "$local_build/registry" ]]; then
    build_root="$(cd "$local_build" && pwd -P)"
    rust_dir="$build_root/rust/bin"
    cargo_root="$build_root"
fi
if [[ -x "$rust_dir/cargo" ]]; then
    export PATH="$rust_dir:$PATH"
else
    export PATH="$PATH:$HOME/.cargo/bin"
fi
# Xcode's build environment does not include the usual Homebrew bin directories.
export PATH="$PATH:/opt/homebrew/bin:/usr/local/bin"
if ! command -v cmake >/dev/null 2>&1; then
    echo "error: CMake is required to build SDL3." >&2
    exit 1
fi

offline=()
if [[ -d "$cargo_root/registry" ]]; then
    export CARGO_HOME="$cargo_root"
    offline=(--offline)
fi
export CARGO_TARGET_DIR="$build_root/$cache_folder"
export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-17.0}"
export MACOSX_DEPLOYMENT_TARGET=13.0
export SDKROOT="$(xcrun --sdk "$platform" --show-sdk-path)"

cd "$project"
if [[ ${#offline[@]} -eq 0 ]]; then
    cargo build --release --locked --target "$target"
else
    cargo build --release --locked --offline --target "$target"
fi
mkdir -p "$(dirname "$destination")"
install -m 0755 "$CARGO_TARGET_DIR/$target/release/rustyzoo" "$destination"

# The local Xcode prototype can include the owner’s installed data for the first run.
if [[ ${RUSTYZOO_EMBED_GAME_DATA:-NO} == YES ]]; then
    game_name=Zoo-Tycoon-Complete-Collection
    game_source="$workspace/$game_name"
    game_destination="$(dirname "$destination")/$game_name"
    if [[ -f "$workspace/ipad-game-data/$game_name/ZOO.INI" ]]; then
        game_source="$workspace/ipad-game-data/$game_name"
    fi
    if [[ -f "$game_source/ZOO.INI" && ! -f "$game_destination/.rustyzoo-import-complete" ]]; then
        mkdir -p "$game_destination"
        rsync -a "$game_source/" "$game_destination/"
        touch "$game_destination/.rustyzoo-import-complete"
    fi
fi
