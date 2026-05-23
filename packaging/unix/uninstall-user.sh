#!/usr/bin/env sh
set -eu

APP_NAME=claudie
PREFIX="${PREFIX:-"$HOME/.local"}"
BIN_DIR="$PREFIX/bin"
LIB_DIR="$PREFIX/share/$APP_NAME"
BIN="$BIN_DIR/$APP_NAME"

if [ -x "$BIN" ]; then
  "$BIN" --uninstall-claude-hooks --quiet || true
else
  claudie --uninstall-claude-hooks --quiet || true
fi

rm -f "$BIN"
rm -rf "$LIB_DIR"

printf '%s\n' "Removed $APP_NAME from $PREFIX"
