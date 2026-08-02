#!/bin/bash
# Interactive, guided provisioning of one capability secret. Vaultwarden (via
# the official `bw` CLI, upstreams.toml [bitwarden-cli]) is the canonical
# store — not per-machine Keychain, so a secret survives a reinstall or a
# second machine the same way any other vault item does. The capability's
# env file still gets the one required plaintext copy (containers read
# plaintext env, not the vault), and axon-overlay/secrets/*.md gets a
# pointer, never the value. Generalizes the manual dance vaultwarden's own
# ADMIN_TOKEN went through by hand once — the second capability needing the
# same steps (postgres) is the signal to make it a real tool. Run this
# yourself in your own terminal (it prompts for your vault master password);
# it never prints the secret value or the vault session key anywhere.
set -euo pipefail

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
source "$TOOLS_DIR/lib/paths.sh"
source "$TOOLS_DIR/lib/toml.sh"

usage() {
  echo "usage: setup-secret.sh <capability> <slug> <ENV_VAR_NAME>" >&2
  echo "  e.g. setup-secret.sh postgres password POSTGRES_PASSWORD" >&2
  exit 1
}
[ $# -eq 3 ] || usage
CAP="$1" SLUG="$2" VAR="$3"
SERVICE="$CAP-$SLUG"

command -v bw >/dev/null 2>&1 || {
  case "$(uname -s)" in
    Darwin) BW_INSTALL="brew install bitwarden-cli" ;;
    *)      BW_INSTALL="snap install bw, or see bitwarden.com/help/cli" ;;
  esac
  echo "setup-secret.sh: no 'bw' (Bitwarden CLI) on PATH — $BW_INSTALL (see upstreams.toml [bitwarden-cli])" >&2
  exit 1
}
command -v jq >/dev/null 2>&1 || { echo "setup-secret.sh: no 'jq' on PATH" >&2; exit 1; }

VW_ENV="$AXON_PERSONAL_ROOT/config/vaultwarden.env"
[ -f "$VW_ENV" ] || {
  echo "setup-secret.sh: no $VW_ENV — vaultwarden is the canonical secret store here," >&2
  echo "set it up first (capabilities/vaultwarden/README.md)." >&2
  exit 1
}
DOMAIN="$(grep -m1 '^DOMAIN=' "$VW_ENV" | cut -d= -f2-)"
[ -n "$DOMAIN" ] || { echo "setup-secret.sh: $VW_ENV has no DOMAIN=" >&2; exit 1; }

MANIFEST="$AXON_ROOT/capabilities/$CAP/service.toml"
if [ -f "$MANIFEST" ]; then
  ENV_FILE_REL="$(toml_get env_file "$MANIFEST")"
else
  ENV_FILE_REL="config/$CAP.env"
  echo "setup-secret.sh: no $MANIFEST, defaulting env file to $ENV_FILE_REL" >&2
fi
ENV_FILE="$AXON_PERSONAL_ROOT/$ENV_FILE_REL"
POINTER="$AXON_PERSONAL_ROOT/secrets/$SERVICE.md"

echo "capability:  $CAP"
echo "vault item:  \"$SERVICE\" (folder: Axon, on $DOMAIN)"
echo "env file:    $ENV_FILE ($VAR)"
echo "pointer doc: $POINTER"
echo

# --- vault session: reuse an inherited $BW_SESSION, else unlock interactively ---
STATUS_JSON="$(bw status 2>/dev/null || true)"
CURRENT_SERVER="$(echo "$STATUS_JSON" | jq -r '.serverUrl // empty')"
[ "$CURRENT_SERVER" = "$DOMAIN" ] || bw config server "$DOMAIN" >/dev/null

VAULT_STATUS="$(bw status 2>/dev/null | jq -r '.status // "unauthenticated"')"
LOCKED_BY_US=0
case "$VAULT_STATUS" in
  unauthenticated)
    echo "setup-secret.sh: not logged in to $DOMAIN yet." >&2
    echo "Run 'bw login' yourself first (email + master password + 2FA if enabled), then re-run this script." >&2
    exit 1
    ;;
  locked)
    echo "Vault is locked — enter your master password to unlock it for this run only"
    echo "(never captured by this script; only the resulting session key is held, in memory, for this run):"
    BW_SESSION="$(bw unlock --raw)" || { echo "setup-secret.sh: unlock failed" >&2; exit 1; }
    export BW_SESSION
    LOCKED_BY_US=1
    ;;
  unlocked) ;; # $BW_SESSION already valid in the inherited environment
  *)
    echo "setup-secret.sh: unexpected vault status \"$VAULT_STATUS\"" >&2
    exit 1
    ;;
esac

cleanup() {
  [ "$LOCKED_BY_US" = 1 ] && bw lock >/dev/null 2>&1 || true
  unset VALUE BW_SESSION
}
trap cleanup EXIT

bw sync --session "$BW_SESSION" >/dev/null

FOLDER_ID="$(bw list folders --session "$BW_SESSION" --nointeraction \
  | jq -r '.[] | select(.name=="Axon") | .id' | head -1)"
if [ -z "$FOLDER_ID" ]; then
  FOLDER_ID="$(bw get template folder --session "$BW_SESSION" --nointeraction \
    | jq '.name="Axon"' | bw encode \
    | bw create folder --session "$BW_SESSION" --nointeraction | jq -r .id)"
fi

ITEM_ID="$(bw list items --folderid "$FOLDER_ID" --session "$BW_SESSION" --nointeraction \
  | jq -r --arg n "$SERVICE" '.[] | select(.name==$n) | .id' | head -1)"

if [ -n "$ITEM_ID" ]; then
  echo "A vault item \"$SERVICE\" already exists."
  read -r -p "[K]eep it and just re-sync the env file / [R]egenerate random / [E]nter a new value / [C]ancel: " CHOICE
else
  echo "No vault item \"$SERVICE\" yet."
  read -r -p "[G]enerate a random value (recommended) / [E]nter your own / [C]ancel: " CHOICE
fi

VALUE=""
case "${CHOICE:0:1}" in
  [Kk])
    VALUE="$(bw get notes "$ITEM_ID" --session "$BW_SESSION" --nointeraction)"
    ;;
  [Gg]|[Rr])
    VALUE="$(openssl rand -base64 32)"
    ;;
  [Ee])
    read -r -s -p "Paste the value (hidden, not echoed): " VALUE
    echo
    [ -n "$VALUE" ] || { echo "setup-secret.sh: empty value, aborting" >&2; exit 1; }
    ;;
  *)
    echo "cancelled, nothing written"
    exit 0
    ;;
esac

echo
echo "About to:"
[ "${CHOICE:0:1}" != "K" ] && [ "${CHOICE:0:1}" != "k" ] && echo "  - write the value to the vault item \"$SERVICE\" (folder Axon)"
echo "  - write $VAR into $ENV_FILE"
echo "  - write/update $POINTER (pointer only, never the value)"
read -r -p "Continue? [y/N]: " CONFIRM
case "$CONFIRM" in
  [Yy]*) ;;
  *) echo "cancelled, nothing written"; exit 0 ;;
esac

if [ "${CHOICE:0:1}" != "K" ] && [ "${CHOICE:0:1}" != "k" ]; then
  if [ -n "$ITEM_ID" ]; then
    ENCODED="$(bw get item "$ITEM_ID" --session "$BW_SESSION" --nointeraction \
      | jq --arg notes "$VALUE" '.notes=$notes' | bw encode)"
    bw edit item "$ITEM_ID" "$ENCODED" --session "$BW_SESSION" --nointeraction >/dev/null
  else
    ENCODED="$(bw get template item --session "$BW_SESSION" --nointeraction \
      | jq --arg name "$SERVICE" --arg notes "$VALUE" --arg fid "$FOLDER_ID" \
          '.type=2 | .name=$name | .notes=$notes | .folderId=$fid | .secureNote={type:0} | del(.login,.card,.identity)' \
      | bw encode)"
    bw create item "$ENCODED" --session "$BW_SESSION" --nointeraction >/dev/null
  fi
fi

mkdir -p "$(dirname "$ENV_FILE")"
touch "$ENV_FILE"
if grep -q "^$VAR=" "$ENV_FILE" 2>/dev/null; then
  # grep -v + rewrite, not sed substitution — VALUE can contain regex/sed-delimiter
  # characters (base64 includes +/=) that would otherwise need fragile escaping.
  TMP="$ENV_FILE.tmp.$$"
  grep -v "^$VAR=" "$ENV_FILE" > "$TMP" || true
  printf '%s=%s\n' "$VAR" "$VALUE" >> "$TMP"
  mv "$TMP" "$ENV_FILE"
else
  printf '%s=%s\n' "$VAR" "$VALUE" >> "$ENV_FILE"
fi

mkdir -p "$(dirname "$POINTER")"
cat > "$POINTER" <<EOF
# $SERVICE

Reference only — the plaintext value never touches a file, per \`axon-overlay/README.md\`'s rule
(\`$ENV_FILE_REL\` is the one required exception: the container needs a real env value, and
\`*.env\` is hard-blocked in \`.gitignore\` so it never reaches git).

- **Plaintext:** Vaultwarden, item \`$SERVICE\`, folder \`Axon\`, on \`$DOMAIN\`.
  Retrieve: \`bw unlock\` (or reuse \`\$BW_SESSION\`), then
  \`bw get notes "$SERVICE" --session "\$BW_SESSION"\`
- **Deployed as:** \`$ENV_FILE_REL\` → \`$VAR\`
- **Provisioned via:** \`tools/setup-secret.sh $CAP $SLUG $VAR\`
EOF

echo
echo "done — $VAR written to $ENV_FILE, vault item \"$SERVICE\" set, pointer doc updated."
