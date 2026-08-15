#!/bin/sh
set -eu

LABEL="com.glenzli.pcp-console"
DOMAIN="gui/$(id -u)"
SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
PROJECT_ROOT="$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)"
PCP_HOME="${PCP_HOME:-$HOME/Library/Application Support/PCP}"
BIN_DIR="$PROJECT_ROOT/target/release"
CONSOLE_BINARY="$BIN_DIR/pcp-console"
RUNTIME_BINARY="$BIN_DIR/pcp-runtime"
PLIST_DIR="$HOME/Library/LaunchAgents"
PLIST_PATH="$PLIST_DIR/$LABEL.plist"
LOG_DIR="$PCP_HOME/logs"
TEMPLATE="$PROJECT_ROOT/packaging/macos/$LABEL.plist.in"

escape_replacement() {
  printf '%s' "$1" | sed 's/[\\&|]/\\&/g'
}

mkdir -p "$PCP_HOME" "$LOG_DIR" "$PLIST_DIR"
chmod 700 "$PCP_HOME" "$LOG_DIR"

cargo build --release -p pcp-console -p pcp-runtime --manifest-path "$PROJECT_ROOT/Cargo.toml"

temporary="$(mktemp "$PLIST_DIR/$LABEL.XXXXXX")"
trap 'rm -f "$temporary"' EXIT
sed \
  -e "s|@PCP_CONSOLE_BINARY@|$(escape_replacement "$CONSOLE_BINARY")|g" \
  -e "s|@PCP_RUNTIME_BINARY@|$(escape_replacement "$RUNTIME_BINARY")|g" \
  -e "s|@PCP_HOME@|$(escape_replacement "$PCP_HOME")|g" \
  -e "s|@PCP_PROJECT_ROOT@|$(escape_replacement "$PROJECT_ROOT")|g" \
  -e "s|@LOG_DIR@|$(escape_replacement "$LOG_DIR")|g" \
  "$TEMPLATE" >"$temporary"
plutil -lint "$temporary" >/dev/null
chmod 600 "$temporary"

launchctl bootout "$DOMAIN/$LABEL" >/dev/null 2>&1 || true
mv "$temporary" "$PLIST_PATH"
trap - EXIT
launchctl bootstrap "$DOMAIN" "$PLIST_PATH"
launchctl enable "$DOMAIN/$LABEL"
launchctl kickstart -k "$DOMAIN/$LABEL"

echo "PCP Console is managed by $LABEL at http://127.0.0.1:4318/"
echo "PCP home: $PCP_HOME"
