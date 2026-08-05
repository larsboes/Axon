import { execFileSync, execSync } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { sveltekit } from "@sveltejs/kit/vite";
import { defineConfig, type Plugin, type ProxyOptions } from "vite";
import {
  hasSameOrigin,
  installCommsProxyAuthorization,
  isMutation,
  loadCommsProxyCredential,
} from "./vite/comms-proxy-auth";

const AXON_ROOT = resolve(fileURLToPath(new URL(".", import.meta.url)), "..");
const port = Number(process.env.AXON_PORT ?? 47117);
const APP_BUNDLE_LIMIT_BYTES = 500_000;
const MAPLIBRE_BUNDLE_LIMIT_BYTES = 1_100_000;

interface RegistryEntry {
  name: string;
  kind: string;
  scope: "capability" | "spine";
  port: string;
  proxy_extra: string[];
  proxy_api_only: string;
}

function maplibreBundleGuard(): Plugin {
  return {
    name: "maplibre-bundle-guard",
    generateBundle(_options, bundle) {
      const chunks = [];
      for (const output of Object.values(bundle)) {
        if (output.type !== "chunk") continue;
        if (Object.keys(output.modules).some((id) => id.includes("/maplibre-gl/"))) {
          chunks.push(output);
        } else if (Buffer.byteLength(output.code) > APP_BUNDLE_LIMIT_BYTES) {
          this.error(
            `${output.fileName} exceeds the ${APP_BUNDLE_LIMIT_BYTES}-byte application limit.`,
          );
        }
      }
      if (chunks.length === 0) return;

      const eagerChunk = chunks.find((chunk) => !chunk.isDynamicEntry);
      if (eagerChunk) {
        this.error(`MapLibre must remain lazy; ${eagerChunk.fileName} is not a dynamic entry.`);
      }

      const bytes = chunks.reduce((total, chunk) => total + Buffer.byteLength(chunk.code), 0);
      if (bytes > MAPLIBRE_BUNDLE_LIMIT_BYTES) {
        this.error(
          `MapLibre bundles total ${bytes} bytes; the limit is ${MAPLIBRE_BUNDLE_LIMIT_BYTES}.`,
        );
      }
    },
  };
}

// The proxy table is derived, not written. `tools/capability.sh registry` reads the
// service.toml manifests through tools/lib/toml.sh, the shell-side TOML parser
// (README.md#one-manifest-per-concern), so a capability's port is declared in exactly one file and this config,
// axon-status and the runner all read the same number.
//
// Read once at dev-server start: enabling a capability or moving a port means
// restarting the dashboard, which is honest — the shell's shape follows the machine's.
function registry(): RegistryEntry[] {
  const out = execFileSync(resolve(AXON_ROOT, "tools/capability.sh"), ["registry"], {
    encoding: "utf8",
  });
  return JSON.parse(out) as RegistryEntry[];
}

function buildProxy(): Record<string, ProxyOptions> {
  const proxy: Record<string, ProxyOptions> = {};
  const commsCredential = loadCommsProxyCredential(AXON_ROOT);

  if (!commsCredential.authorization) {
    console.warn(
      `[dashboard] Comms write proxy has no credential (${commsCredential.reason}); write routes remain fail-closed.`,
    );
  }

  for (const svc of registry()) {
    // The spine is this process; a capability with no port has no HTTP surface.
    if (svc.scope === "spine" || !svc.port) continue;
    const target = `http://127.0.0.1:${svc.port}`;

    // Uniform rule, no manifest field needed: /<name> reaches the capability with the
    // prefix stripped, so a capability's own contract never has to know it is proxied.
    const proxyPath =
      svc.proxy_api_only === "true" ? `/${svc.name}/api` : `/${svc.name}`;
    const options: ProxyOptions = {
      target,
      changeOrigin: true,
      rewrite: (path) => path.replace(new RegExp(`^/${svc.name}`), ""),
    };
    if (svc.name === "comms" && commsCredential.authorization) {
      const authorization = commsCredential.authorization;
      options.configure = (server) => installCommsProxyAuthorization(server, authorization);
    }
    proxy[proxyPath] = options;

    // Surfaces whose paths predate that rule (transit's /api, scouting's /discover)
    // pass through unstripped, declared per capability in its own manifest.
    for (const extra of svc.proxy_extra ?? []) {
      proxy[extra] = { target, changeOrigin: true };
    }
  }

  // The real LifeOS Life Dashboard (~/.claude/LIFEOS/PULSE/pulse.ts), an external
  // system this shell links to and never an Axon capability. Not in the registry, so
  // it stays declared here by hand.
  proxy["/lifeos-pulse"] = {
    target: "http://localhost:31337",
    changeOrigin: true,
    rewrite: (path) => path.replace(/^\/lifeos-pulse/, ""),
  };

  // macmon used to be hand-proxied here, on a hardcoded 9911, because it was not a
  // capability. It is capabilities/macmon now, so the registry loop above already gives it
  // /macmon from its own `port` — one fewer place for that number to be wrong.

  return proxy;
}

/** Top memory consumers on this machine, as JSON. Runs `ps` every request, so this
 *  is intentionally not a capability — it is a trivial dev-server convenience for the
 *  dashboard's Systems page hover detail. The response is ~500 bytes, uncached.
 *  
 *  Each entry: { pid, rss_mb, name }. `name` is the basename of the executable. */
function topProcesses(_req: import("http").IncomingMessage, res: import("http").ServerResponse) {
  try {
    const raw = execSync("ps -eo pid=,rss=,comm=", { encoding: "utf8", timeout: 3000 });
    const lines = raw.trim().split("\n");
    const procs = lines
      .map((l) => {
        const m = l.trim().match(/^(\d+)\s+(\d+)\s+(.+)$/);
        if (!m) return null;
        const rssKb = parseInt(m[2], 10);
        // Filter out the ps process itself and tiny system processes
        if (rssKb < 100 * 1024) return null; // < 100 MB
        return { pid: parseInt(m[1], 10), rss_mb: Math.round(rssKb / 1024), name: m[3].split("/").pop() ?? m[3] };
      })
      .filter((p): p is NonNullable<typeof p> => p !== null)
      .sort((a, b) => b.rss_mb - a.rss_mb);
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(JSON.stringify(procs.slice(0, 15)));
  } catch {
    res.writeHead(500);
    res.end(JSON.stringify({ error: "failed to read processes" }));
  }
}

function guardCommsMutations(
  req: import("http").IncomingMessage,
  res: import("http").ServerResponse,
  next: () => void,
) {
  if (!req.url?.startsWith("/comms/") || !isMutation(req.method) || hasSameOrigin(req.headers)) {
    next();
    return;
  }
  res.writeHead(403, { "Content-Type": "application/json" });
  res.end(JSON.stringify({ error: "cross-origin Comms mutations are not allowed" }));
}

// A function, not an object, so the proxy table is built only when a server is
// actually starting. `buildProxy()` shells out to tools/capability.sh and reads
// every manifest in the repo; a production build has no server, no need for the
// table, and — under Bazel — neither the script nor the manifests in its sandbox.
// Evaluating it at config load was what kept this app out of the build graph.
export default defineConfig(({ command }) => ({
  plugins: [sveltekit(), maplibreBundleGuard(), {
    name: "top-processes",
    configureServer(server) {
      server.middlewares.use(guardCommsMutations);
      server.middlewares.use("/api/top-processes", topProcesses);
    },
  }],
  server: {
    host: "127.0.0.1",
    port,
    strictPort: true,
    ...(command === "serve" ? { proxy: buildProxy() } : {}),
  },
  preview: { host: "127.0.0.1", port, strictPort: true },
  build: {
    // The plugin keeps ordinary chunks at Vite's default budget and gives only the
    // separately guarded async map module its measured allowance.
    chunkSizeWarningLimit: MAPLIBRE_BUNDLE_LIMIT_BYTES / 1000,
  },
}));
