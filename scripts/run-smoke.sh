#!/usr/bin/env bash
# Start a fresh Whisper relay and run the smoke suite against it.
#
# Usage: bash scripts/run-smoke.sh
# CI runs this on Linux; it builds only whisper-relay (no desktop deps).

set -euo pipefail

# Repo root, resolved from the script location (robust regardless of CWD).
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT/server"

cargo build --release -p whisper-relay

SCRATCH="$(mktemp -d)"
export WHISPER_DB_PATH="$SCRATCH/smoke.db"
# Burst budget shared by every per-IP bucket. Must clear the ~27 group
# operations (ownership transfer included) while still letting the
# 120-envelope burst test hit its rate limit.
export WHISPER_RATE_BURST=40
export WHISPER_RATE_REFILL=0

"$ROOT/target/release/whisper-relay" &
RELAY_PID=$!
# Cleanup must never fail the script: on Windows a just-killed relay can still
# hold the SQLite file for a moment, so the rm is best-effort.
trap 'kill "$RELAY_PID" 2>/dev/null || true; rm -rf "$SCRATCH" 2>/dev/null || true' EXIT

# Wait for the relay to accept connections.
for _ in $(seq 1 20); do
  if curl -fsS "http://127.0.0.1:8080/healthz" >/dev/null 2>&1; then
    break
  fi
  sleep 0.5
done

node tests/smoke.mjs
