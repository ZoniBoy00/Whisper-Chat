#!/usr/bin/env bash
# Start a fresh Whisper relay and run the smoke suite against it.
#
# Usage: bash scripts/run-smoke.sh
# CI runs this on Linux; it builds only whisper-relay (no desktop deps).

set -euo pipefail

cd "$(dirname "$0")/../server"

cargo build --release -p whisper-relay

SCRATCH="$(mktemp -d)"
export WHISPER_DB_PATH="$SCRATCH/smoke.db"
export WHISPER_RATE_BURST=20
export WHISPER_RATE_REFILL=0

"$OLDPWD/../target/release/whisper-relay" &
RELAY_PID=$!
trap 'kill "$RELAY_PID" 2>/dev/null || true; rm -rf "$SCRATCH"' EXIT

# Wait for the relay to accept connections.
for _ in $(seq 1 20); do
  if curl -fsS "http://127.0.0.1:8080/healthz" >/dev/null 2>&1; then
    break
  fi
  sleep 0.5
done

node tests/smoke.mjs
