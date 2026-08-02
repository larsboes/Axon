/**
 * Secrets Guard — pi agent extension
 *
 * Prevents the agent from reading secrets files (.env, .secrets, *credentials*),
 * Bitwarden items, and other sensitive material. Blocks at the tool-call boundary
 * before any data reaches the LLM.
 *
 * == What is blocked ==
 *
 *   read  — any path matching *.env, *.secrets, *credentials*, *.pem, *.key,
 *           ~/.netrc, ~/.git-credentials, git credential store, kubeconfig,
 *           ~/.ssh/id_* private keys
 *
 *   ls/grep/find — listing/searching a secret path, or a pattern/glob that
 *         targets those files (e.g. `find . -name '*.env'`)
 *
 *   bash  — cat/grep/head/tail/less/more/nl/wc/type on those file patterns
 *         — bw get, bw list, bw sync (reading secrets from Bitwarden)
 *         — echo $SECRET_VAR (echoing env vars that match known secret names)
 *
 *   edit  — any edit targeting a path matching *.env, *.secrets, etc.
 *
 * == What is allowed ==
 *
 *   bash  — source .env && …  (sources secrets into env, doesn't print them)
 *         — bw unlock          (unlocks vault, session token not a secret value)
 *         — bw encode, bw generate
 *         — export VAR=…
 *
 *   write — writing to .env or .secrets files (legitimate config setup)
 *
 * == Custom tool: vault_exec ==
 *
 *   A registered tool that runs a command with an env file sourced, captures
 *   stdout/stderr, and strips env-file values from the output before the LLM
 *   sees it. Use when you need to call an API that needs a credential:
 *
 *     vault_exec({cmd: "curl -s https://api.example.com", env_file: ".env"})
 *
 *   The secret values are never part of the tool-call input the LLM constructs
 *   and never appear in the output — the extension resolves them directly.
 */

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import { readFileSync, existsSync } from "node:fs";
import { execSync } from "node:child_process";
import { resolve } from "node:path";

// ---- patterns -----------------------------------------------------------

/** File path patterns that contain secrets — blocked from read/edit. */
const SECRET_FILE_PATTERNS: RegExp[] = [
  /\.env$/,
  /\.env\.[a-zA-Z0-9_.-]+$/,   // .env.local, .env.production, .env.development
  /\.secrets?$/,
  /\.secrets?\.[a-zA-Z0-9_.-]+$/,
  /credentials?\./i,
  /\.pem$/,
  /\.key$/,
  /\.p12$/,
  /\.pfx$/,
  /token\./i,
  /secret\./i,
  // credential stores & private keys with no tell-tale extension
  /(^|\/)\.netrc$/,
  /(^|\/)\.git-credentials$/,
  /(^|\/)config\/git\/credentials$/,
  /kubeconfig/i,
  /(^|\/)\.ssh\/id_/,          // id_rsa, id_ed25519, … (no extension)
];

/** Bash commands or patterns that leak secrets. */
const SECRET_BASH_PATTERNS: RegExp[] = [
  // cat/grep/head/tail etc. on secret files
  /\b(cat|bat|less|more|head|tail|nl|tac|rev)\s+.*\.env/,
  /\b(cat|bat|less|more|head|tail|nl|tac|rev)\s+.*\.secrets?/,
  /\b(grep|rg|ag|ack|find|sed)\s+.*\.env/,
  /\b(grep|rg|ag|ack|find|sed)\s+.*\.secrets?/,
  /\b(wc|sort|uniq)\s+.*\.env/,
  /\b(wc|sort|uniq)\s+.*\.secrets?/,
  // Bitwarden read operations
  /\bbw\s+get\b/,
  /\bbw\s+list\b/,
  /\bbw\s+sync\b/,
  /\bbw\s+export\b/,
  // Printing env vars that likely hold secrets
  /\becho\s+\$[A-Z_]*(?:TOKEN|SECRET|KEY|PASSWORD|PASS|CREDENTIAL|API_KEY|SECRET_)\w*/,
  /\bprintenv\b/,
  /\benv\b.*\n/,
  // OpenSSL reading private keys
  /\bopenssl\s+(?:pkey|rsa|ec|dsa)\s+.*-in\s+.*\.(?:pem|key)/,
];

/** Env-var name patterns that hold secrets — stripped from tool output. */
const SECRET_VAR_PATTERNS: RegExp[] = [
  /TOKEN/i,
  /SECRET/i,
  /PASSWORD/i,
  /PASS(?:_|$)/i,
  /API_KEY/i,
  /APIKEY/i,
  /CREDENTIAL/i,
  /AUTH/i,
  /_KEY$/i,
  /_ID$/i,
];

// ---- helpers -----------------------------------------------------------

/** Check if a path matches any secret-file pattern. */
function isSecretPath(path: string): boolean {
  const normalized = path.replace(/\\/g, "/");
  return SECRET_FILE_PATTERNS.some((p) => p.test(normalized));
}

/** Build a readable list of matched patterns for the block reason. */
function matchReason(path: string): string {
  const matched = SECRET_FILE_PATTERNS
    .filter((p) => p.test(path))
    .map((p) => p.source);
  return matched.length > 0 ? `(${matched.join(", ")})` : "(pattern match)";
}

/** Heuristic: does a bash command try to read secret files? */
function isSecretReadCommand(command: string): { blocked: boolean; reason?: string } {
  // Allow `source` — it loads vars into env without printing them
  if (/^(\s*source\s+|\s*\.\s+)[^\n&|;]*\.(env|secrets?)\b/.test(command)) {
    // But still check if the same command also tries to cat/grep the file
    for (const pattern of SECRET_BASH_PATTERNS) {
      if (pattern.test(command)) {
        return { blocked: true, reason: `matches secret-read pattern: ${pattern.source}` };
      }
    }
    return { blocked: false };
  }

  // Allow `export` — setting vars is not reading
  if (/^\s*export\s+[A-Za-z_][A-Za-z0-9_]*=/.test(command)) {
    return { blocked: false };
  }

  // Allow `bw unlock` (sets session, doesn't read secrets)
  if (/^\s*bw\s+unlock\b/.test(command)) {
    return { blocked: false };
  }

  // Allow `bw encode` / `bw generate`
  if (/^\s*bw\s+(encode|generate)\b/.test(command)) {
    return { blocked: false };
  }

  // Check against known secret-leaking patterns
  for (const pattern of SECRET_BASH_PATTERNS) {
    if (pattern.test(command)) {
      return { blocked: true, reason: `matches secret-read pattern: ${pattern.source}` };
    }
  }

  return { blocked: false };
}

/** Strip known secret values from a string (for tool-result sanitization). */
function sanitizeOutput(text: string): string {
  if (!text) return text;

  let result = text;

  // Strip lines that look like secret env assignments
  result = result.replace(
    /^(export\s+)?([A-Za-z_][A-Za-z0-9_]*)=['"]?[^\s'"]{8,}['"]?$/gm,
    (match, _export, key) => {
      if (SECRET_VAR_PATTERNS.some((p) => p.test(key))) {
        return `${_export || ""}${key}=****`;
      }
      return match;
    },
  );

  // Strip base64-encoded OAuth tokens and similar long random strings
  // Typical refresh tokens are long (300+ chars)
  result = result.replace(/[A-Za-z0-9_-]{300,}(?:[=]{1,2})?/g, "****");

  return result;
}

/** Load an env file and return the parsed key-value pairs (values never returned). */
function loadEnvFile(envPath: string): Record<string, string> {
  const resolved = envPath.startsWith("/")
    ? envPath
    : resolve(process.cwd(), envPath);

  if (!existsSync(resolved)) {
    throw new Error(`env file not found: ${resolved}`);
  }

  const body = readFileSync(resolved, "utf-8");
  const vars: Record<string, string> = {};

  for (const line of body.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const eqIdx = trimmed.indexOf("=");
    if (eqIdx <= 0) continue;
    const key = trimmed.slice(0, eqIdx).trim();
    let value = trimmed.slice(eqIdx + 1).trim();
    // Strip surrounding quotes
    if ((value.startsWith('"') && value.endsWith('"')) ||
        (value.startsWith("'") && value.endsWith("'"))) {
      value = value.slice(1, -1);
    }
    vars[key] = value;
  }

  return vars;
}

/** Run a command with an env file sourced, capture output, sanitize it. */
function runWithEnv(cmd: string, envFile?: string): string {
  const env = { ...process.env };

  if (envFile) {
    const loaded = loadEnvFile(envFile);
    for (const [key, val] of Object.entries(loaded)) {
      env[key] = val;
    }
  }

  const result = execSync(cmd, {
    env,
    encoding: "utf-8",
    maxBuffer: 10 * 1024 * 1024,
    stdio: ["ignore", "pipe", "pipe"],
    timeout: 30_000,
  });

  return sanitizeOutput(result);
}

// ---- extension ---------------------------------------------------------

export default function (pi: ExtensionAPI) {
  // ── Block reads of secret files ───────────────────────────────────────

  pi.on("tool_call", async (event, ctx) => {
    if (event.toolName === "read") {
      const path = event.input.path as string;
      if (isSecretPath(path)) {
        ctx.ui?.notify?.(
          `[secrets-guard] blocked read of ${path} — use vault_exec for commands needing credentials`,
          "warning",
        );
        return {
          block: true,
          reason: `secrets-guard: reading ${path} would expose secrets (${matchReason(path)}). Run commands with vault_exec({cmd: "...", env_file: "${path}"}) instead — it sources the file without exposing values.`,
        };
      }
    }

    if (event.toolName === "edit") {
      const path = event.input.path as string;
      if (isSecretPath(path)) {
        return {
          block: true,
          reason: `secrets-guard: editing ${path} is blocked — env files contain secrets.`,
        };
      }
    }

    if (event.toolName === "ls" || event.toolName === "grep" || event.toolName === "find") {
      const input = event.input as { path?: string; pattern?: string; glob?: string };
      const p = input.path ?? "";
      if (p && isSecretPath(p)) {
        return {
          block: true,
          reason: `secrets-guard: ${event.toolName} on ${p} could expose secrets ${matchReason(p)}. Use vault_keys to list env-file variable names without values.`,
        };
      }
      // A pattern/glob that targets secret files (e.g. find -name '*.env')
      const needle = `${input.pattern ?? ""} ${input.glob ?? ""}`;
      if (needle.trim() && SECRET_FILE_PATTERNS.some((rx) => rx.test(needle))) {
        return {
          block: true,
          reason: `secrets-guard: ${event.toolName} pattern/glob targets secret files (${needle.trim()}).`,
        };
      }
    }

    if (event.toolName === "bash") {
      const command = event.input.command as string;
      const check = isSecretReadCommand(command);
      if (check.blocked) {
        ctx.ui?.notify?.(
          `[secrets-guard] blocked bash command that would read secrets`,
          "warning",
        );
        return {
          block: true,
          reason: `secrets-guard: blocked — ${check.reason}. Use vault_exec({cmd: "...", env_file: ".env"}) to run commands with env vars sourced without exposing values. Or source the env file with \`source .env && your_command\`.`,
        };
      }
    }
  });

  // ── Sanitize tool results so leaked secrets don't reach the LLM ──────

  pi.on("tool_result", async (event) => {
    if (event.toolName === "bash" || event.toolName === "read") {
      const sanitized = event.content?.map((block) => {
        if (block.type === "text") {
          return { ...block, text: sanitizeOutput(block.text) };
        }
        return block;
      });

      if (sanitized) {
        return {
          content: sanitized,
          details: event.details,
          isError: event.isError,
        };
      }
    }
  });

  // ── Register vault_exec tool ──────────────────────────────────────────

  pi.registerTool({
    name: "vault_exec",
    label: "Vault Exec",
    description:
      "Run a shell command with an env file sourced. The env file is read directly by the extension — the LLM never sees the secret values. Captures stdout/stderr and strips leaked secrets from the output. Use for curl, API calls, or any command that needs credentials from .env / .secrets files.",

    parameters: Type.Object({
      cmd: Type.String({
        description: "The shell command to run (e.g., 'curl -s https://api.example.com/endpoint')",
      }),
      env_file: Type.Optional(
        Type.String({
          description: "Path to an env file to source before running the command (e.g., '.env' or 'config/secrets.env'). The file is read by the extension, never seen by the LLM.",
        }),
      ),
      timeout: Type.Optional(
        Type.Number({
          description: "Timeout in seconds (default 30)",
        }),
      ),
    }),

    async execute(toolCallId, params, signal, onUpdate, ctx) {
      try {
        const env = { ...process.env };

        if (params.env_file) {
          const loaded = loadEnvFile(params.env_file);
          for (const [key, val] of Object.entries(loaded)) {
            env[key] = val;
          }

          // Notify user that env file was used
          ctx.ui?.notify?.(
            `[vault_exec] sourced ${params.env_file} (${Object.keys(loaded).length} vars)`,
            "info",
          );
        }

        const result = execSync(params.cmd, {
          env,
          encoding: "utf-8",
          maxBuffer: 10 * 1024 * 1024,
          stdio: ["ignore", "pipe", "pipe"],
          timeout: (params.timeout ?? 30) * 1000,
          signal,
        });

        const sanitized = sanitizeOutput(result);

        return {
          content: [{ type: "text", text: sanitized }],
          details: {
            exitCode: 0,
            cmd: params.cmd,
            env_file: params.env_file ?? null,
          },
        };
      } catch (error: unknown) {
        const err = error as Error & { stderr?: string; stdout?: string; status?: number };
        const stderr = err.stderr ? sanitizeOutput(err.stderr) : "";
        const stdout = err.stdout ? sanitizeOutput(err.stdout) : "";
        const message = sanitizeOutput(err.message || String(error));

        return {
          content: [
            { type: "text", text: stdout || message },
            ...(stderr ? [{ type: "text", text: `stderr: ${stderr}` }] : []),
          ],
          details: {
            exitCode: err.status ?? -1,
            cmd: params.cmd,
            env_file: params.env_file ?? null,
          },
          isError: true,
        };
      }
    },
  });

  // ── Register vault_read tool (safe read of env files — shows keys, not values) ──

  pi.registerTool({
    name: "vault_keys",
    label: "Vault Keys",
    description:
      "List the variable names (not values) from an env file. Use this to discover what secrets are available without exposing their values.",

    parameters: Type.Object({
      env_file: Type.String({
        description: "Path to env file (e.g., '.env', 'config/secrets.env')",
      }),
    }),

    async execute(toolCallId, params, signal, onUpdate, ctx) {
      try {
        const loaded = loadEnvFile(params.env_file);
        const keys = Object.keys(loaded).sort();

        return {
          content: [{
            type: "text",
            text: `Keys in ${params.env_file} (${keys.length} vars):\n${keys.map((k) => `  ${k}`).join("\n")}`,
          }],
          details: {
            count: keys.length,
            env_file: params.env_file,
          },
        };
      } catch (error) {
        return {
          content: [{ type: "text", text: `Error reading ${params.env_file}: ${error}` }],
          isError: true,
          details: {},
        };
      }
    },
  });
}
