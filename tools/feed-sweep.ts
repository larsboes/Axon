// One pass of the Feed source collector, then exit.
//
// The sweep itself has always existed as comms' `POST /sources/scan`, and until now the only
// thing that ever called it was a button in the dashboard — so on a machine where nobody opened
// the dashboard, nothing was collected and every source's `last_run_at` quietly aged. This is
// the caller that runs without anyone watching: `capabilities/feed-sweep/service.toml` carries
// the interval as `schedule`, and tools/service-runner.sh renders that to launchd or systemd.
//
// A job rather than a fifth tokio ticker inside comms-server. The drains beside it are fast,
// local and stateful; a collector that walks the open web is slow, allowed to fail, and keeps
// nothing between runs — and giving comms-server a fourth reason to be up is the opposite of
// what an on-demand capability is for.
//
// It talks to comms over HTTP and never to comms' database. That is the documented composition
// edge (README.md#schemas-and-dependency-direction): a capability depends on another's contract,
// never its code. It is also what keeps a second process out of that store — `Store::open` runs
// the whole migration on every call, and two openers doing that concurrently deadlock on the
// table locks a no-op `ALTER TABLE` still takes.

import { existsSync, readFileSync } from "node:fs";
import { isAbsolute, join } from "node:path";

import { axonRoot, overlayRoot } from "./lib/overlay.ts";

function fail(message: string): never {
  console.error(`feed-sweep: ${message}`);
  process.exit(1);
}

// `bun run tools/feed-sweep.ts` directly, not a bash launcher that execs bun — which is the
// convention every bun capability already follows (capabilities/knowledge-graph). It is also
// load-bearing rather than cosmetic: `service-runner.sh persistence_path_dirs` puts the
// INTERPRETER's directory on the generated unit's PATH by resolving `command[0]`, and a launcher
// script hides bun from that resolution. The first scheduled run through launchd died on
// `exec: bun: not found`, exit 127 — the same defect the watchdog units carried from 2026-07-09
// to 2026-07-25, arrived at from the other direction.
const AXON_ROOT = axonRoot();
const OVERLAY = overlayRoot(AXON_ROOT);
if (!OVERLAY) fail("no 'overlay' in axon.local.toml or axon.toml — run tools/install.sh");

/** comms' port, from the one file that declares it. Never a literal here — the manifest is the
 *  single home for that number, and the registry, the proxy and the health poll all read it. */
function commsPort(): string {
  const manifest = join(AXON_ROOT, "capabilities", "comms", "service.toml");
  if (!existsSync(manifest)) fail(`no ${manifest}`);
  const line = readFileSync(manifest, "utf8")
    .split("\n")
    .find((l) => /^port\s*=/.test(l));
  const port = line?.match(/"([^"]*)"/)?.[1] ?? "";
  if (!port) fail(`no port in ${manifest}`);
  return port;
}

/** The token for comms' mutating routes, resolved the way the Rust server and the dashboard's
 *  Vite proxy resolve it: comms.json names a file, the file holds the secret. It is read into
 *  memory and sent as a header — never as an argument, where `ps` would show it, and never in
 *  the URL, which leaks to logs, history and referrers. */
function bearerToken(): string {
  const configPath = join(OVERLAY, "config", "comms.json");
  if (!existsSync(configPath)) fail(`no ${configPath}`);
  let secretRef: unknown;
  try {
    secretRef = (JSON.parse(readFileSync(configPath, "utf8")) as Record<string, unknown>).api_secret_file;
  } catch (error) {
    fail(`could not read ${configPath}: ${error}`);
  }
  if (typeof secretRef !== "string" || !secretRef.trim()) {
    fail("comms.json has no api_secret_file — /sources/scan rejects every request without one");
  }
  let path = secretRef.trim();
  if (path.startsWith("~/")) path = join(process.env.HOME ?? "", path.slice(2));
  else if (!isAbsolute(path)) path = join(AXON_ROOT, path);
  let token: string;
  try {
    token = readFileSync(path, "utf8").trim();
  } catch {
    // The path is named, the contents never are. A message that quoted the file would be one
    // `cat` away from a message that quoted the secret.
    fail("cannot read the comms API secret file named by comms.json");
  }
  if (!token) fail("the comms API secret file is empty");
  return token;
}

interface SourceResult {
  source_id?: string;
  adapter?: string;
  discovered?: number;
  fetched?: number;
  new_count?: number;
  failed?: string[];
  /** Present only when the source itself could not be collected — comms keeps going and reports
   *  the reason per source rather than failing the whole run. */
  error?: string;
}

const port = commsPort();
const url = `http://127.0.0.1:${port}/sources/scan`;

let response: Response;
try {
  response = await fetch(url, {
    method: "POST",
    headers: {
      // Header, not query string (OPERATIONAL_RULES).
      Authorization: `Bearer ${bearerToken()}`,
      "Content-Type": "application/json",
    },
    // An empty body scans every ENABLED source. Which sources are enabled is comms'
    // configuration to state, so this job never names one.
    body: "{}",
    signal: AbortSignal.timeout(300_000),
  });
} catch (error) {
  // comms being down is the expected failure: it is an on-demand capability and this job runs on
  // a timer that knows nothing about that. Reported and non-zero, never swallowed: a schedule
  // that silently does nothing is indistinguishable from one that is working.
  fail(`comms is not answering on 127.0.0.1:${port} — ${error}`);
}

const body = await response.text();
if (!response.ok) {
  // comms' own error string, which is about a feed source and never about a credential.
  fail(`/sources/scan answered ${response.status}: ${body.slice(0, 400)}`);
}

let parsed: unknown;
try {
  parsed = JSON.parse(body);
} catch {
  fail(`/sources/scan answered 200 with a body that is not JSON: ${body.slice(0, 200)}`);
}

// `sources`, matching the key comms' handler actually emits — read off the live response, not
// guessed from the handler's local variable name, which is `results`.
const payload = parsed as { sources?: unknown; new_count?: number };
const results: SourceResult[] = Array.isArray(payload.sources) ? (payload.sources as SourceResult[]) : [];

// One line per source and a total, so a run that found nothing reads differently from a run that
// never happened — which is the whole reason this job exists.
let broken = 0;
for (const row of results) {
  // A source that could not be reached at all reports its reason and nothing else. Without this
  // it renders as `discovered=0 fetched=0`, which is exactly what a source with nothing new
  // looks like — and the difference between "quiet" and "broken" is the only thing a log nobody
  // reads until something is wrong actually needs to say.
  if (row.error) {
    broken += 1;
    console.error(`feed-sweep: ${row.source_id ?? "?"} FAILED — ${row.error}`);
    continue;
  }
  console.log(
    `feed-sweep: ${row.source_id ?? "?"} discovered=${row.discovered ?? 0} ` +
      `fetched=${row.fetched ?? 0} new=${row.new_count ?? 0} failed=${row.failed?.length ?? 0}`,
  );
}
console.log(
  `feed-sweep: ${results.length} source(s), ${payload.new_count ?? 0} new item(s)` +
    (broken ? `, ${broken} source(s) unreachable` : ""),
);
