#!/usr/bin/env sh
set -eu

APP_NAME=claudie
PREFIX="${PREFIX:-"$HOME/.local"}"
BIN_DIR="$PREFIX/bin"
LIB_DIR="$PREFIX/share/$APP_NAME"
SOURCE_BIN="${SOURCE_BIN:-"./target/release/claudie"}"

mkdir -p "$BIN_DIR" "$LIB_DIR/assets"
cp "$SOURCE_BIN" "$BIN_DIR/$APP_NAME"
chmod 755 "$BIN_DIR/$APP_NAME"

if [ -d "./assets/claudie" ]; then
  rm -rf "$LIB_DIR/assets/claudie"
  mkdir -p "$LIB_DIR/assets"
  cp -R "./assets/claudie" "$LIB_DIR/assets/claudie"
fi

"$BIN_DIR/$APP_NAME" --install-claude-hooks --quiet

printf '%s\n' "Installed $APP_NAME to $BIN_DIR/$APP_NAME"
printf '%s\n' "Make sure $BIN_DIR is on PATH before starting Claude Code."
