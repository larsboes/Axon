#!/usr/bin/env bun
/**
 * Axon · comms capability — one-time Google OAuth bootstrap.
 *
 * Reads the Desktop OAuth client JSON from Bitwarden/Vaultwarden (bw-native,
 * never hardcoded), runs a loopback consent flow for the Gmail + Calendar
 * scopes the comms pipeline needs, and writes the resulting refresh token into
 * the axon-overlay overlay (config/comms.env, git-ignored via *.env). The
 * refresh token is never printed to the console.
 *
 * Run once — interactive, opens a browser for consent:
 *   bw unlock                 # then: export BW_SESSION="..."
 *   bun capabilities/comms/auth/get-refresh-token.ts
 */

import { readFileSync, writeFileSync, mkdirSync, chmodSync } from "node:fs";
import { join } from "node:path";
import { overlayRoot } from "../../../libs/overlay/overlay.ts";

const BW_ITEM = "Axon Google OAuth 2.0 Client IDs";
const SCOPES = [
  "https://www.googleapis.com/auth/gmail.modify",
  "https://www.googleapis.com/auth/calendar.events",
];
const PORT = 8765;
const REDIRECT = `http://localhost:${PORT}`;

function die(msg: string): never {
  console.error(`✗ ${msg}`);
  process.exit(1);
}

/** Fetch the OAuth client credentials from Bitwarden (bw-native, no shell interpolation). */
async function bwClient(): Promise<{ client_id: string; client_secret: string; token_uri: string }> {
  if (!process.env.BW_SESSION) die("BW_SESSION not set — run `bw unlock` and export it first");
  const proc = Bun.spawn(["bw", "get", "item", BW_ITEM], { env: process.env, stdout: "pipe", stderr: "pipe" });
  const out = await new Response(proc.stdout).text();
  if ((await proc.exited) !== 0) die(`\`bw get item "${BW_ITEM}"\` failed — is the folder/item name exact and the vault unlocked?`);

  const item = JSON.parse(out);
  const candidates: string[] = [];
  if (item.notes) candidates.push(item.notes);
  for (const f of item.fields ?? []) if (f?.value) candidates.push(f.value);

  for (const c of candidates) {
    const start = c.indexOf("{");
    if (start < 0) continue;
    try {
      const j = JSON.parse(c.slice(start));
      const inst = j.installed ?? j.web ?? j;
      if (inst.client_id && inst.client_secret) {
        return {
          client_id: inst.client_id,
          client_secret: inst.client_secret,
          token_uri: inst.token_uri ?? "https://oauth2.googleapis.com/token",
        };
      }
    } catch {
      /* try the next candidate */
    }
  }
  die(`could not find the OAuth client JSON inside the "${BW_ITEM}" item (checked notes + custom fields)`);
}

/** Upsert a KEY=value line in an env-file body. */
function upsertEnv(env: string, key: string, value: string): string {
  if (!/^[A-Z][A-Z0-9_]*$/.test(key)) throw new Error(`invalid env key: ${key}`);
  const prefix = `${key}=`;
  let replaced = false;
  const lines = env.split("\n").map((line) => {
    if (!replaced && line.startsWith(prefix)) {
      replaced = true;
      return `${key}=${value}`;
    }
    return line;
  });
  if (replaced) return lines.join("\n");
  const sep = env === "" || env.endsWith("\n") ? "" : "\n";
  return `${env}${sep}${key}=${value}\n`;
}

// --- 1. credentials from Bitwarden ---
const { client_id, client_secret, token_uri } = await bwClient();

// --- 2. build the consent URL (offline + forced consent → guarantees a refresh_token) ---
const authUrl =
  "https://accounts.google.com/o/oauth2/v2/auth?" +
  new URLSearchParams({
    client_id,
    redirect_uri: REDIRECT,
    response_type: "code",
    scope: SCOPES.join(" "),
    access_type: "offline",
    prompt: "consent",
  }).toString();

console.log("→ Opening the Google consent screen in your browser…");
console.log(`  If it doesn't open, paste this URL manually:\n  ${authUrl}\n`);

// --- 3. catch the loopback redirect ---
const code = await new Promise<string>((resolve) => {
  const server = Bun.serve({
    port: PORT,
    fetch(req) {
      const url = new URL(req.url);
      const err = url.searchParams.get("error");
      if (err) die(`consent denied: ${err}`);
      const c = url.searchParams.get("code");
      if (c) {
        resolve(c);
        setTimeout(() => server.stop(), 150);
        return new Response("✅ Axon: consent received — you can close this tab.", {
          headers: { "content-type": "text/plain; charset=utf-8" },
        });
      }
      return new Response("waiting for ?code…", { status: 400 });
    },
  });
});

Bun.spawn(["open", authUrl]); // macOS launcher

// --- 4. exchange the code for tokens ---
console.log("→ Exchanging authorization code for tokens…");
const res = await fetch(token_uri, {
  method: "POST",
  headers: { "content-type": "application/x-www-form-urlencoded" },
  body: new URLSearchParams({
    code,
    client_id,
    client_secret,
    redirect_uri: REDIRECT,
    grant_type: "authorization_code",
  }).toString(),
});
const tok = (await res.json()) as { refresh_token?: string; error?: string; error_description?: string };
if (!res.ok || !tok.refresh_token) {
  die(
    `token exchange failed: ${JSON.stringify(tok)}` +
      (tok.refresh_token ? "" : "\n(no refresh_token — revoke the prior grant at myaccount.google.com and re-run; prompt=consent is set)"),
  );
}

// --- 5. persist into the overlay (git-ignored *.env), never printed ---
const selectedOverlay = overlayRoot();
if (!selectedOverlay) die("could not resolve the deployment overlay; run tools/install.sh or set AXON_OVERLAY_ROOT");
const cfgDir = join(selectedOverlay, "config");
mkdirSync(cfgDir, { recursive: true });
const envPath = join(cfgDir, "comms.env");
let env = "";
try {
  env = readFileSync(envPath, "utf8");
} catch {
  /* new file */
}
env = upsertEnv(env, "GOOGLE_CLIENT_ID", client_id);
env = upsertEnv(env, "GOOGLE_CLIENT_SECRET", client_secret);
env = upsertEnv(env, "GOOGLE_REFRESH_TOKEN", tok.refresh_token);
writeFileSync(envPath, env, { mode: 0o600 });
chmodSync(envPath, 0o600);

console.log(`✅ Refresh token stored (not printed) → ${envPath}`);
console.log("   Scopes: gmail.modify + calendar.events. Gmail+Calendar prereq done.");
