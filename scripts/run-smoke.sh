#!/usr/bin/env bash
# Start a fresh Whisper relay and run the smoke suite against it.
#
# Usage: bash scripts/run-smoke.sh
# CI runs this on Linux; it builds only whisper-relay (no desktop deps).
#
# The relay port is configurable via WHISPER_SMOKE_PORT. When unset, an
# ephemeral localhost port is selected so the smoke suite cannot collide with
# a production relay or another local service.

set -euo pipefail

# Repo root, resolved from the script location (robust regardless of CWD).
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT/server"

cargo build --release -p whisper-relay

# Fresh scratch dir on EVERY run, so re-runs never trip over duplicates or
# rate limits from the previous relay's database.
SCRATCH="$(mktemp -d)"
export WHISPER_DB_PATH="$SCRATCH/smoke.db"
# Burst budget shared by every per-IP bucket. Must clear the ~27 group
# operations (ownership transfer included) while still letting the
# 120-envelope burst test hit its rate limit.
export WHISPER_RATE_BURST=40
export WHISPER_RATE_REFILL=0

if [ -n "${WHISPER_SMOKE_PORT:-}" ]; then
  PORT="$WHISPER_SMOKE_PORT"
else
  PORT="$(python3 - <<'PY'
import socket

with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
)"
fi

python3 - "$PORT" <<'PY'
import socket
import sys

port = int(sys.argv[1])
with socket.socket() as sock:
    try:
        sock.bind(("127.0.0.1", port))
    except OSError as exc:
        raise SystemExit(f"smoke port {port} is unavailable: {exc}")
PY

export WHISPER_ADDR="0.0.0.0:$PORT"
export WHISPER_WS_URL="ws://127.0.0.1:$PORT/ws"
export WHISPER_HTTP_URL="http://127.0.0.1:$PORT"

"$ROOT/target/release/whisper-relay" &
RELAY_PID=$!
# Cleanup must never fail the script: on Windows a just-killed relay can still
# hold the SQLite file for a moment, so the rm is best-effort.
trap 'kill "$RELAY_PID" 2>/dev/null || true; rm -rf "$SCRATCH" 2>/dev/null || true' EXIT

READY=0
# Wait for the relay to accept connections.
for _ in $(seq 1 20); do
  if curl -fsS "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1; then
    READY=1
    break
  fi
  if ! kill -0 "$RELAY_PID" 2>/dev/null; then
    echo "SMOKE TEST ERROR: relay exited before readiness" >&2
    exit 1
  fi
  sleep 0.5
done

if [ "$READY" -ne 1 ]; then
  echo "SMOKE TEST ERROR: relay did not become ready on port $PORT" >&2
  exit 1
fi

node tests/smoke.mjs
