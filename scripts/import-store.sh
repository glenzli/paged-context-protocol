#!/bin/sh
set -eu

usage() {
  echo "usage: sh scripts/import-store.sh --source <context.sqlite3> [--enrollment-state <pcp-enrollments.json>] [--home <PCP home>]" >&2
  exit 2
}

SOURCE=""
ENROLLMENT_STATE=""
PCP_HOME="${PCP_HOME:-$HOME/Library/Application Support/PCP}"
while [ "$#" -gt 0 ]; do
  case "$1" in
    --source) SOURCE="${2:-}"; shift 2 ;;
    --enrollment-state) ENROLLMENT_STATE="${2:-}"; shift 2 ;;
    --home) PCP_HOME="${2:-}"; shift 2 ;;
    *) usage ;;
  esac
done
[ -n "$SOURCE" ] || usage
[ -f "$SOURCE" ] || { echo "source Store is not a regular file: $SOURCE" >&2; exit 1; }
command -v sqlite3 >/dev/null || { echo "sqlite3 is required for a consistent SQLite import" >&2; exit 1; }

DATA_DIR="$PCP_HOME/data"
DESTINATION="$DATA_DIR/context.sqlite3"
mkdir -p "$DATA_DIR"
chmod 700 "$PCP_HOME" "$DATA_DIR"
[ ! -e "$DESTINATION" ] || { echo "refusing to overwrite existing PCP Store: $DESTINATION" >&2; exit 1; }

sqlite3 "$SOURCE" ".backup '$DESTINATION'"
[ "$(sqlite3 "$DESTINATION" 'PRAGMA quick_check;')" = "ok" ] || {
  rm -f "$DESTINATION"
  echo "imported Store failed SQLite quick_check" >&2
  exit 1
}
chmod 600 "$DESTINATION"

if [ -n "$ENROLLMENT_STATE" ]; then
  [ -f "$ENROLLMENT_STATE" ] || { echo "enrollment state is not a regular file: $ENROLLMENT_STATE" >&2; exit 1; }
  install -m 600 "$ENROLLMENT_STATE" "$DATA_DIR/pcp-enrollments.json"
fi

echo "Imported PCP Store into $DESTINATION"
echo "Start PCP Console after configuring any tenant-specific maintenance worker in $PCP_HOME/config/runtime.toml"
