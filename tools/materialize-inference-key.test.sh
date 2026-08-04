#!/bin/bash
set -euo pipefail

if [ -n "${TEST_SRCDIR:-}" ] && [ -n "${TEST_WORKSPACE:-}" ]; then
  ROOT="$TEST_SRCDIR/$TEST_WORKSPACE"
else
  ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fi
TEST_ROOT="$(mktemp -d)"
trap 'rm -rf "$TEST_ROOT"' EXIT
OVERLAY="$TEST_ROOT/overlay"
FAKE_BIN="$TEST_ROOT/bin"
mkdir -p "$OVERLAY/config" "$FAKE_BIN"
printf 'DOMAIN=https://vault.test\n' > "$OVERLAY/config/vaultwarden.env"

cat > "$FAKE_BIN/bw" <<'EOF'
#!/bin/bash
case "$1 $2" in
  "status ") printf '{"status":"%s","serverUrl":"%s"}\n' "${BW_FAKE_STATUS:-unlocked}" "${BW_FAKE_SERVER:-https://vault.test}" ;;
  "sync --session") ;;
  "list folders") printf '[{"id":"folder-1","name":"Axon"}]\n' ;;
  "list items") printf '[{"id":"item-1","name":"inference-gemini-api-key"}]\n' ;;
  "get item") printf '{"id":"item-1","type":2}\n' ;;
  "get notes") printf 'synthetic-test-key\n' ;;
  *) echo "unexpected bw call: $*" >&2; exit 1 ;;
esac
EOF
chmod +x "$FAKE_BIN/bw"

OUTPUT="$(
  PATH="$FAKE_BIN:$PATH" \
  AXON_ROOT="$ROOT" \
  AXON_INFERENCE_KEY_OVERLAY="$OVERLAY" \
  BW_SESSION="synthetic-session" \
  "$ROOT/tools/materialize-inference-key" gemini
)"

TARGET="$OVERLAY/config/runtime-secrets/inference-gemini-api-key"
[ "$(cat "$TARGET")" = "synthetic-test-key" ]
case "$(uname -s)" in
  Darwin) MODE="$(stat -f '%Lp' "$TARGET")" ;;
  *) MODE="$(stat -c '%a' "$TARGET")" ;;
esac
[ "$MODE" = "600" ]
case "$OUTPUT" in
  *synthetic-test-key*) echo "FAIL: command output exposed the key" >&2; exit 1 ;;
esac
grep -q "credential materialized" <<< "$OUTPUT"

MISMATCH_ERROR="$TEST_ROOT/mismatch.err"
if PATH="$FAKE_BIN:$PATH" AXON_ROOT="$ROOT" AXON_INFERENCE_KEY_OVERLAY="$OVERLAY" \
  BW_SESSION="synthetic-session" BW_FAKE_SERVER="https://other.test" \
  "$ROOT/tools/materialize-inference-key" gemini >/dev/null 2>"$MISMATCH_ERROR"; then
  echo "FAIL: a logged-in session on another server must stop before config changes" >&2
  exit 1
fi
grep -q "run 'bw logout' yourself" "$MISMATCH_ERROR"

if PATH="$FAKE_BIN:$PATH" AXON_ROOT="$ROOT" AXON_INFERENCE_KEY_OVERLAY="$OVERLAY" \
  BW_SESSION="synthetic-session" "$ROOT/tools/materialize-inference-key" unknown \
  >/dev/null 2>&1; then
  echo "FAIL: unknown providers must be rejected" >&2
  exit 1
fi

echo "materialize-inference-key tests passed"
