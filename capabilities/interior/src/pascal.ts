/**
 * MCP client for the Pascal 3D editor, over either transport.
 *
 * Which one matters, and the difference is not cosmetic:
 *
 * The standalone `@pascal-app/mcp` package spawned over stdio has no storage layer, so scene
 * geometry works but anything that persists — `save_scene`, `create_project` — dies with
 * `undefined is not an object (evaluating 'this.withWriteTransaction')`. It is the right
 * transport for headless `build` and `check`, which need no database.
 *
 * The editor's own MCP server, started by `@pascal-app/cli` and advertised in
 * `pascal status --json`, shares the SQLite store the browser reads, so a scene saved through
 * it appears in an open tab. That is the transport for anything the eye is meant to see. It is
 * stateless over HTTP and accepts repeat initialises, unlike a hand-started `--http` server,
 * which allows exactly one per process and answers later ones with -32600.
 */

import { PASCAL_PKG } from "./paths.ts";

type Pending = { resolve: (v: unknown) => void; reject: (e: Error) => void };

export interface Tool {
  name: string;
  description?: string;
  inputSchema?: unknown;
}

interface Transport {
  rpc(method: string, params?: unknown, notify?: boolean): Promise<any>;
  close(): void;
}

/** One server process, ours, no database. */
class StdioTransport implements Transport {
  #proc: Bun.Subprocess<"pipe", "pipe", "pipe">;
  #pending = new Map<number, Pending>();
  #buf = "";
  #nextId = 1;

  constructor(scene?: string) {
    const args = ["--bun", PASCAL_PKG, "--stdio"];
    if (scene) args.push("--scene", scene);
    this.#proc = Bun.spawn(["bunx", ...args], {
      stdin: "pipe",
      stdout: "pipe",
      stderr: "pipe",
      env: {
        ...process.env,
        PASCAL_DATA_DIR: process.env.PASCAL_DATA_DIR ?? `${process.env.HOME}/.pascal/data`,
      },
    });
    this.#pump();
  }

  async #pump() {
    for await (const chunk of this.#proc.stdout) {
      this.#buf += new TextDecoder().decode(chunk);
      let nl: number;
      while ((nl = this.#buf.indexOf("\n")) >= 0) {
        const line = this.#buf.slice(0, nl).trim();
        this.#buf = this.#buf.slice(nl + 1);
        if (!line) continue;
        let msg: any;
        try {
          msg = JSON.parse(line);
        } catch {
          continue; // banner or log line, not protocol
        }
        const p = this.#pending.get(msg.id);
        if (!p) continue;
        this.#pending.delete(msg.id);
        msg.error ? p.reject(new Error(JSON.stringify(msg.error))) : p.resolve(msg.result);
      }
    }
    for (const [, p] of this.#pending) p.reject(new Error("pascal-mcp exited before answering"));
    this.#pending.clear();
  }

  rpc(method: string, params?: unknown, notify = false): Promise<any> {
    if (notify) {
      this.#write({ jsonrpc: "2.0", method, params });
      return Promise.resolve(null);
    }
    const id = this.#nextId++;
    const p = new Promise<any>((res, rej) => this.#pending.set(id, { resolve: res, reject: rej }));
    this.#write({ jsonrpc: "2.0", id, method, params });
    return p;
  }

  #write(body: Record<string, unknown>) {
    this.#proc.stdin.write(JSON.stringify(body) + "\n");
    this.#proc.stdin.flush();
  }

  close() {
    this.#proc.kill();
  }
}

/** The editor's server, shares the store the browser reads. */
class HttpTransport implements Transport {
  #url: string;
  #token: string | null;
  #session: string | null = null;
  #nextId = 1;

  constructor(url: string, token: string | null) {
    this.#url = url;
    this.#token = token;
  }

  async rpc(method: string, params?: unknown, notify = false): Promise<any> {
    const headers: Record<string, string> = {
      "Content-Type": "application/json",
      Accept: "application/json, text/event-stream",
    };
    // Header, never a query parameter: URLs end up in logs, history and referrers.
    if (this.#token) headers.Authorization = `Bearer ${this.#token}`;
    if (this.#session) headers["mcp-session-id"] = this.#session;
    const body: Record<string, unknown> = { jsonrpc: "2.0", method };
    if (params !== undefined) body.params = params;
    if (!notify) body.id = this.#nextId++;

    const res = await fetch(this.#url, { method: "POST", headers, body: JSON.stringify(body) });
    const sid = res.headers.get("mcp-session-id");
    if (sid) this.#session = sid;
    const text = await res.text();
    if (notify) return null;

    // Answers arrive as a single SSE frame, or as bare JSON.
    const line = text.split("\n").find((l) => l.startsWith("data: "));
    const msg = JSON.parse(line ? line.slice(6) : text);
    if (msg.error) throw new Error(JSON.stringify(msg.error));
    return msg.result;
  }

  close() {}
}

/**
 * The editor's MCP server is authenticated. The CLI writes its bearer token to
 * `~/.pascal/run/mcp-token` at startup; it is read here and never logged or printed.
 */
async function mcpToken(): Promise<string | null> {
  const path = `${process.env.PASCAL_HOME ?? `${process.env.HOME}/.pascal`}/run/mcp-token`;
  const f = Bun.file(path);
  return (await f.exists()) ? (await f.text()).trim() : null;
}

/** Ask the CLI where the editor and its MCP server are; ports are chosen at startup. */
export async function editorEndpoints(): Promise<{ editor: string; mcp: string } | null> {
  try {
    const proc = Bun.spawn(["bunx", "--bun", "@pascal-app/cli", "status", "--json"], {
      stdout: "pipe",
      stderr: "ignore",
    });
    const status = JSON.parse(await new Response(proc.stdout).text());
    if (!status?.running || !status?.state?.url || !status?.state?.mcp?.url) return null;
    return { editor: status.state.url, mcp: status.state.mcp.url };
  } catch {
    return null;
  }
}

export class Pascal {
  #t: Transport;
  /** True when talking to the running editor, so saves are visible in the browser. */
  readonly live: boolean;

  private constructor(t: Transport, live: boolean) {
    this.#t = t;
    this.live = live;
  }

  /**
   * `needsStore` asks for the editor's server, which is the only one that can persist a scene.
   * Without it, or when the editor is not running, this spawns its own headless server.
   */
  static async open(opts: { scene?: string; needsStore?: boolean } = {}): Promise<Pascal> {
    let transport: Transport | null = null;
    let live = false;
    if (opts.needsStore) {
      const ends = await editorEndpoints();
      if (ends) {
        transport = new HttpTransport(ends.mcp, await mcpToken());
        live = true;
      }
    }
    transport ??= new StdioTransport(opts.scene);

    const p = new Pascal(transport, live);
    await p.#t.rpc("initialize", {
      protocolVersion: "2025-06-18",
      capabilities: {},
      clientInfo: { name: "axon-interior", version: "0.1.0" },
    });
    await p.#t.rpc("notifications/initialized", undefined, true);
    return p;
  }

  async tools(): Promise<Tool[]> {
    return (await this.#t.rpc("tools/list", {})).tools;
  }

  /** Tool payloads arrive as a JSON string inside a text content block. */
  async call(name: string, args: Record<string, unknown> = {}): Promise<any> {
    const r = await this.#t.rpc("tools/call", { name, arguments: args });
    const text = r?.content?.find((c: any) => c.type === "text")?.text;
    if (r?.isError) throw new Error(`${name}: ${text}`);
    try {
      return text ? JSON.parse(text) : r;
    } catch {
      return text ?? r;
    }
  }

  close() {
    this.#t.close();
  }
}
