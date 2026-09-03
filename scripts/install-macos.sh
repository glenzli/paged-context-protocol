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
MCP_BINARY="$BIN_DIR/pcp-mcp"
INSTALLED_BIN_DIR="$PCP_HOME/bin"
INSTALLED_MCP_BINARY="$INSTALLED_BIN_DIR/pcp-mcp"
CHATGPT_LAUNCHER_SOURCE="$PROJECT_ROOT/integrations/chatgpt/launch-pcp-mcp"
INSTALLED_CHATGPT_LAUNCHER="$INSTALLED_BIN_DIR/pcp-chatgpt-mcp"
PLIST_DIR="$HOME/Library/LaunchAgents"
PLIST_PATH="$PLIST_DIR/$LABEL.plist"
LOG_DIR="$PCP_HOME/logs"
TEMPLATE="$PROJECT_ROOT/packaging/macos/$LABEL.plist.in"

escape_replacement() {
  printf '%s' "$1" | sed 's/[\\&|]/\\&/g'
}

wait_for_console_exit() {
  attempt=0
  while curl -fsS http://127.0.0.1:4318/api/health >/dev/null 2>&1; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 50 ]; then
      echo "PCP Console did not release port 4318 after launchd bootout" >&2
      return 1
    fi
    sleep 0.1
  done
}

bootstrap_console() {
  attempt=0
  while ! launchctl bootstrap "$DOMAIN" "$PLIST_PATH"; do
    # launchd can report an error after accepting the job. Do not turn that race into a
    # duplicate bootstrap, and otherwise give a just-removed label a bounded time to settle.
    if launchctl print "$DOMAIN/$LABEL" >/dev/null 2>&1; then
      return 0
    fi
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 50 ]; then
      echo "PCP Console could not be loaded after $attempt attempts" >&2
      return 1
    fi
    sleep 0.1
  done
}

mkdir -p "$PCP_HOME" "$INSTALLED_BIN_DIR" "$LOG_DIR" "$PLIST_DIR"
chmod 700 "$PCP_HOME" "$INSTALLED_BIN_DIR" "$LOG_DIR"

cargo build --release -p pcp-console -p pcp-runtime -p pcp-mcp --manifest-path "$PROJECT_ROOT/Cargo.toml"

installed_mcp_temporary="$(mktemp "$INSTALLED_BIN_DIR/.pcp-mcp.XXXXXX")"
trap 'rm -f "$installed_mcp_temporary"' EXIT
cp "$MCP_BINARY" "$installed_mcp_temporary"
chmod 700 "$installed_mcp_temporary"
mv "$installed_mcp_temporary" "$INSTALLED_MCP_BINARY"
trap - EXIT

installed_chatgpt_temporary="$(mktemp "$INSTALLED_BIN_DIR/.pcp-chatgpt-mcp.XXXXXX")"
trap 'rm -f "$installed_chatgpt_temporary"' EXIT
cp "$CHATGPT_LAUNCHER_SOURCE" "$installed_chatgpt_temporary"
chmod 700 "$installed_chatgpt_temporary"
mv "$installed_chatgpt_temporary" "$INSTALLED_CHATGPT_LAUNCHER"
trap - EXIT

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
wait_for_console_exit
mv "$temporary" "$PLIST_PATH"
trap - EXIT
bootstrap_console
launchctl enable "$DOMAIN/$LABEL"
launchctl kickstart -k "$DOMAIN/$LABEL"

echo "PCP Console is managed by $LABEL at http://127.0.0.1:4318/"
echo "PCP home: $PCP_HOME"
echo "ChatGPT MCP launcher: $INSTALLED_CHATGPT_LAUNCHER"
