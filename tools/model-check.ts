// Is every model this machine names still real, still served, and still the newest of its line?
//
// Renovate watches package registries; a model id is not a package, so nothing watched these.
// The cost of that was measured on 2026-08-30: `summarization_light` named `apple-on-device`
// months after apfel replaced the server that answered to it, and every unattended request 404'd
// with `model_not_found` — at request time, so it read as "the light rung never summarizes
// anything" rather than as a typo.
//
// Three questions, deliberately separate, because each has a different answer and a different
// cost:
//
//   1. Is the declared model in the provider's catalogue?  Absent = broken now.
//   2. Does it actually answer?  (--probe)  Listed is not served: `gemini-3.7-flash` is in the
//      catalogue and returns 503 "high demand" on the free tier, three attempts running.
//   3. Is something newer available in the same family?  Advisory, never a failure — newer is a
//      decision about quality, cost and capacity, and question 2 is why it cannot be automatic.
//
// Not wired into `tools/doctor`: this dials third-party APIs and spends quota, which no check
// that runs on every invocation should do. `doctor --online` is the sibling that probes declared
// endpoints; this one is run when a model is being chosen or reviewed.

import { readFileSync } from "node:fs";
import { join } from "node:path";

import { axonRoot, overlayRoot } from "./lib/overlay.ts";

interface Backend {
  api?: string;
  base_url?: string;
  api_key_file?: string;
}
interface Role {
  backend?: string;
  model?: string;
}
interface InferenceConfig {
  backends?: Record<string, Backend>;
  roles?: Record<string, Role>;
}

const PROBE = process.argv.includes("--probe");

const AXON_ROOT = axonRoot();
const OVERLAY = overlayRoot(AXON_ROOT);
if (!OVERLAY) {
  console.error("model-check: no overlay — run tools/install.sh");
  process.exit(2);
}
const CONFIG_DIR = join(OVERLAY, "config");
const configPath = join(CONFIG_DIR, "inference.json");

let config: InferenceConfig;
try {
  config = JSON.parse(readFileSync(configPath, "utf8"));
} catch (error) {
  console.error(`model-check: cannot read ${configPath}: ${error}`);
  process.exit(2);
}

/** The key file is named relative to the CONFIG directory, the way libs/inference resolves it
 *  (`lib.rs`, `config_directory.join(raw)`). Reading it from the overlay root instead is how a
 *  present credential gets reported as missing — which it was, on 2026-08-30. */
function apiKey(backend: Backend): string | undefined {
  if (!backend.api_key_file) return undefined;
  const path = backend.api_key_file.startsWith("/")
    ? backend.api_key_file
    : join(CONFIG_DIR, backend.api_key_file);
  try {
    return readFileSync(path, "utf8").trim() || undefined;
  } catch {
    return undefined;
  }
}

/** Same-line comparison, on the numeric part of a family name.
 *
 *  `gemini-3.6-flash` and `gemini-3.7-flash` are the same line at different versions;
 *  `nemotron-3-nano` and `nemotron-3-super` are NOT — they are size tiers, and calling a bigger
 *  one "newer" would advise an upgrade nobody asked for onto a model with different economics.
 *  So the family is the id with its version numbers removed, and only an id that matches the
 *  family exactly, differing in those numbers, can be newer. */
function family(id: string): string {
  return id.replace(/[0-9]+(\.[0-9]+)*/g, "#");
}
function versionKey(id: string): number[] {
  return (id.match(/[0-9]+(\.[0-9]+)*/g) ?? []).flatMap((n) => n.split(".").map(Number));
}
function newer(candidate: string, current: string): boolean {
  const a = versionKey(candidate);
  const b = versionKey(current);
  for (let i = 0; i < Math.max(a.length, b.length); i += 1) {
    const left = a[i] ?? 0;
    const right = b[i] ?? 0;
    if (left !== right) return left > right;
  }
  return false;
}

async function catalogue(backend: Backend, key?: string): Promise<string[] | string> {
  const base = (backend.base_url ?? "").replace(/\/$/, "");
  if (!base) return "no base_url declared";
  const headers: Record<string, string> = {};
  if (key) headers.Authorization = `Bearer ${key}`;
  const url = backend.api === "ollama" ? `${base}/api/tags` : `${base}/models`;
  try {
    const response = await fetch(url, { headers, signal: AbortSignal.timeout(20_000) });
    if (!response.ok) return `catalogue unavailable (HTTP ${response.status})`;
    const body = await response.json();
    if (Array.isArray(body?.data)) {
      return body.data.map((m: { id: string }) => String(m.id).replace(/^models\//, ""));
    }
    if (Array.isArray(body?.models)) {
      return body.models.map((m: { name: string }) => String(m.name));
    }
    return "catalogue in an unrecognised shape";
  } catch {
    return "backend not reachable";
  }
}

/// Ask an embedding model for a vector.
///
/// Question 2 is "does it actually answer", and for a retrieval model the answer is a vector.
/// Asking `bge-m3` to reply "OK" gets an HTTP 4xx and reports a model that is installed, served
/// and correct as broken -- the same false alarm in the opposite direction from the one this
/// tool was written to catch.
async function probeEmbedding(
  backend: Backend,
  model: string,
  key?: string,
): Promise<string> {
  const base = (backend.base_url ?? "").replace(/\/$/, "");
  const headers: Record<string, string> = { "Content-Type": "application/json" };
  if (key) headers.Authorization = `Bearer ${key}`;
  // Two shapes, matching libs/inference `ResolvedRole::embedding_endpoint`: a second opinion
  // about which URL is correct is how this tool starts disagreeing with the thing it checks.
  const ollama = backend.api === "ollama";
  try {
    const response = await fetch(`${base}${ollama ? "/api/embed" : "/embeddings"}`, {
      method: "POST",
      headers,
      body: JSON.stringify(ollama ? { model, input: ["probe"] } : { model, input: "probe" }),
      signal: AbortSignal.timeout(60_000),
    });
    if (!response.ok) return `answers HTTP ${response.status}`;
    const body = await response.json();
    const vector = ollama ? body?.embeddings?.[0] : body?.data?.[0]?.embedding;
    return Array.isArray(vector) && vector.length
      ? `answers (${vector.length}-dimensional)`
      : "answered without a vector";
  } catch {
    return "no answer within 60s";
  }
}

async function probe(backend: Backend, model: string, key?: string): Promise<string> {
  const base = (backend.base_url ?? "").replace(/\/$/, "");
  const headers: Record<string, string> = { "Content-Type": "application/json" };
  if (key) headers.Authorization = `Bearer ${key}`;
  try {
    const response = await fetch(`${base}/chat/completions`, {
      method: "POST",
      headers,
      // Generous, because a reasoning model spends its budget thinking and a stingy cap comes
      // back as `finish_reason: length` — which would make a healthy model look broken here,
      // the mirror of the bug that put 15 chains of thought in the Feed.
      body: JSON.stringify({
        model,
        messages: [{ role: "user", content: "Reply with one word: OK" }],
        max_tokens: 2000,
      }),
      signal: AbortSignal.timeout(60_000),
    });
    if (!response.ok) return `answers HTTP ${response.status}`;
    const body = await response.json();
    return body?.choices?.[0]?.message ? "answers" : "answered without a message";
  } catch {
    return "no answer within 60s";
  }
}

let failures = 0;
const roles = Object.entries(config.roles ?? {});
const cache = new Map<string, string[] | string>();

for (const [roleName, role] of roles) {
  const backendName = role.backend ?? "";
  const backend = config.backends?.[backendName];
  const model = role.model ?? "";
  if (!backend || !model) {
    console.log(`${roleName}: incomplete declaration (backend='${backendName}' model='${model}')`);
    failures += 1;
    continue;
  }
  const key = apiKey(backend);
  if (!cache.has(backendName)) cache.set(backendName, await catalogue(backend, key));
  const listed = cache.get(backendName)!;

  if (typeof listed === "string") {
    // Never a failure. A local backend that is not running and a provider without a catalogue
    // endpoint are both normal, and reporting them as faults is how a check becomes noise.
    console.log(`${roleName}: ${model} on ${backendName} — skipped, ${listed}`);
    continue;
  }

  // Ollama resolves an untagged name to its `:latest` tag: `ollama run bge-m3` and the
  // `bge-m3:latest` its catalogue reports are one model. A role naming the untagged form names
  // it the way the CLI does, so an exact-string test called an installed, answering model
  // missing -- and a check that cries wolf is the one the next person learns to ignore.
  const declaredAs = (id: string): string[] =>
    backend.api === "ollama" && id.endsWith(":latest")
      ? [id, id.slice(0, -":latest".length)]
      : [id];
  const present = listed.some((id) => declaredAs(id).includes(model));
  const candidates = listed.filter((id) => family(id) === family(model) && newer(id, model));
  const suffix = candidates.length ? `  newer available: ${candidates.sort().join(", ")}` : "";

  if (!present) {
    failures += 1;
    const near = listed.filter((id) => family(id) === family(model));
    console.log(
      `${roleName}: ✗ ${model} is NOT in ${backendName}'s catalogue` +
        (near.length ? ` — same family: ${near.sort().join(", ")}` : ""),
    );
    continue;
  }
  // `libs/inference` resolves rungs by name -- `role_on("embedding", ...)` is how a caller asks
  // for the retrieval rung -- so the name is the existing contract for what a role is, and the
  // right question to ask it follows from that rather than from a new field.
  const asksForAVector = /(^|_)embedding(_|$)/.test(roleName);
  const answer = PROBE
    ? ` — ${await (asksForAVector ? probeEmbedding : probe)(backend, model, key)}`
    : "";
  console.log(`${roleName}: ${model} on ${backendName} ok${answer}${suffix}`);
}

console.log(
  `\n${roles.length} role(s) checked${PROBE ? ", each probed" : " (pass --probe to ask each one to answer)"}` +
    `, ${failures} naming a model its backend does not list`,
);
if (failures > 0) process.exit(1);
