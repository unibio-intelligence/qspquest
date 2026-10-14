#!/bin/sh
set -eu
features=sandbox,widget
check_only=0
if [ "$#" -eq 1 ] && [ "$1" = --help ]; then
    printf '%s\n' 'Usage: scripts/build-mcp.sh [--features CSV]' \
      'Features: sandbox, widget, analysis, managed. Default: sandbox,widget.' \
      'Use --features "" for local native stdio without WASM assets.'
    exit 0
fi
if [ "$#" -ne 0 ]; then
    if [ "$#" -ne 2 ] || { [ "$1" != --features ] && [ "$1" != --check-features ]; }; then
        printf '%s\n' 'Use --features CSV or --help' >&2; exit 2
    fi
    features=$2
    if [ "$1" = --check-features ]; then check_only=1; fi
fi
# Refuse unrecognized feature syntax before installing targets or building.
case "$features" in
  ,*|*,|*,,*|*[!a-z,]*) printf '%s\n' 'Invalid feature list' >&2; exit 2 ;;
esac
remaining=$features
while [ -n "$remaining" ]; do
    feature=${remaining%%,*}
    case "$feature" in
      sandbox|widget|analysis|managed) ;;
      *) printf 'Unsupported MCP feature: %s\n' "$feature" >&2; exit 2 ;;
    esac
    case "$remaining" in *,*) remaining=${remaining#*,} ;; *) remaining= ;; esac
done
if [ "$check_only" -eq 1 ]; then exit 0; fi
case ",$features," in
  *,widget,*)
    if ! command -v wasm-bindgen >/dev/null 2>&1; then
      printf '%s\n' 'Widget builds require wasm-bindgen-cli 0.2.129; install with cargo install wasm-bindgen-cli --version 0.2.129 --locked' >&2
      exit 2
    fi
    if [ "$(wasm-bindgen --version)" != 'wasm-bindgen 0.2.129' ]; then
      printf '%s\n' 'Widget builds require wasm-bindgen-cli 0.2.129' >&2; exit 2
    fi
    ;;
esac
case ",$features," in
  *,sandbox,*|*,widget,*) rustup target add wasm32-unknown-unknown ;;
esac
case ",$features," in
  *,sandbox,*)
    cargo build -p qspquest-wasm --release --locked --target wasm32-unknown-unknown --features sandbox-abi
    mkdir -p target/mcp-assets
    cp target/wasm32-unknown-unknown/release/qspquest_wasm.wasm target/mcp-assets/sandbox.wasm
    export QSPQUEST_SANDBOX_WASM="$PWD/target/mcp-assets/sandbox.wasm"
    ;;
esac
case ",$features," in
  *,widget,*)
    cargo build -p qspquest-wasm --release --locked --target wasm32-unknown-unknown
    wasm-bindgen target/wasm32-unknown-unknown/release/qspquest_wasm.wasm --target web --out-dir bindings/wasm/pkg/engine
    ;;
esac
cargo build -p qspquest-mcp --release --locked --features "$features"
