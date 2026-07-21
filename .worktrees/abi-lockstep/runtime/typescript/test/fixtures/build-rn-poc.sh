#!/usr/bin/env bash
# Builds the rn_poc fixture crate's cdylib through the REAL experimental BindingExpansion macro
# path -- the same path `boltffi pack apple`/`android`/`kmp`/`csharp` build their real shipped
# artifacts through (docs/tracks/boltffi-fork.md's CRITICAL naming/calling-convention findings) --
# rather than a plain `cargo build`, which only compiles the narrower stable macro path (no native
# `_panic_message` export, no callback-trait support proven end-to-end).
#
# Not committed as a prebuilt artifact (DISK discipline): this script recompiles on demand, and
# the crate itself is excluded from the workspace so `cargo test --workspace` never touches it.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE_DIR="$SCRIPT_DIR/rn_poc"

export BOLTFFI_BINDING_EXPANSION=1
export BOLTFFI_BINDING_EXPANSION_ROOT="$CRATE_DIR"
export BOLTFFI_BINDING_EXPANSION_SOURCE="$CRATE_DIR/src/lib.rs"
export BOLTFFI_BINDING_EXPANSION_SURFACE=native

cd "$CRATE_DIR"
cargo rustc --lib --release -- --cfg boltffi_binding_expansion

case "$(uname -s)" in
  Darwin) EXT=dylib ;;
  Linux) EXT=so ;;
  *) echo "unsupported platform for rn_poc build" >&2; exit 1 ;;
esac

LIB_PATH="$CRATE_DIR/target/release/librn_poc.$EXT"
if [ ! -f "$LIB_PATH" ]; then
  echo "expected build artifact missing: $LIB_PATH" >&2
  exit 1
fi

echo "$LIB_PATH"
