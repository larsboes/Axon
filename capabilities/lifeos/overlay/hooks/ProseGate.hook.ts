#!/usr/bin/env bun
/**
 * @version 1.0.0
 * ProseGate.hook.ts — route every human-facing document through the writing skills.
 *
 * WHY (2026-07-25): the doctrine "human-facing prose goes through human-writing"
 * is worthless unstated at the moment of writing. The predecessor here,
 * WritingGate.hook.ts, encoded the same lesson after a sponsored LinkedIn post
 * shipped unaudited ("Doctrine existed; nothing enforced it") and then rotted:
 * it was never registered and its _WRITING skill was deleted out from under it.
 * This one hangs off the Write/Edit path that is demonstrably live.
 *
 * TRIGGER: PostToolUse (Write, Edit, MultiEdit)
 *
 * DESIGN:
 *   - ADVISORY ONLY. Emits additionalContext; never blocks, never edits, never
 *     reverts. A gate that fights the writer gets switched off in a week.
 *   - Routes by file type. Prose (.md and friends) -> human-writing's linter.
 *     Source code -> unslop-code's scanner, because code comments are that
 *     skill's domain and human-writing's own description routes code away.
 *   - Speaks only past a threshold: prose must land in a trigger_band
 *     (default strong-tell), code needs a finding at a trigger_severity
 *     (default high). On the calibration corpus 7 of 24 genuine human samples
 *     landed in "watch", so gating there would nag real writing.
 *   - Scope lives in LIFEOS/USER/CONFIG/prose-gate.json, not in this file.
 *   - Fails open on every path: missing config, missing scanner, bad JSON,
 *     timeout. A writing nudge must never be able to break a write.
 */

import { existsSync, readFileSync } from "fs";
import { spawnSync } from "child_process";
import { extname, isAbsolute, join, relative, resolve } from "path";

const HOME = process.env.HOME ?? "";
const CONFIG_PATH = join(HOME, ".claude", "LIFEOS", "USER", "CONFIG", "prose-gate.json");
const SCAN_TIMEOUT_MS = 8000;

const expand = (p: string): string => {
  const expanded = p.startsWith("~/") ? `${HOME}/${p.slice(2)}` : p;
  // Paths come from the operator-owned gate config; call sites still enforce root containment.
  return resolve(expanded); // nosemgrep: javascript.lang.security.audit.path-traversal.path-join-resolve-traversal.path-join-resolve-traversal
};

const isWithin = (root: string, candidate: string): boolean => {
  const rel = relative(root, candidate);
  return rel === "" || (!rel.startsWith("..") && !isAbsolute(rel));
};

/** Minimal glob matcher for the ** / * / ? subset the config uses. */
function globToRe(glob: string): RegExp {
  if (glob.length > 512) return /$a/;
  let re = "";
  for (let i = 0; i < glob.length; i++) {
    const c = glob[i];
    if (c === "*") {
      if (glob[i + 1] === "*") {
        // `**/` may match zero directories, so the slash is part of the option.
        if (glob[i + 2] === "/") { re += "(?:.*/)?"; i += 2; } else { re += ".*"; i += 1; }
      } else re += "[^/]*";
    } else if (c === "?") re += "[^/]";
    else re += c.replace(/[.+^${}()|[\]\\]/g, "\\$&");
  }
  // The bounded converter emits only escaped literals and fixed wildcard fragments.
  return new RegExp(`^${re}$`); // nosemgrep: javascript.lang.security.audit.detect-non-literal-regexp.detect-non-literal-regexp
}

interface Cfg {
  enabled?: boolean;
  roots?: string[];
  exclude_globs?: string[];
  prose?: { extensions?: string[]; scanner?: string; skill?: string; trigger_bands?: string[]; max_findings?: number };
  code?: { extensions?: string[]; scanner?: string; skill?: string; trigger_severities?: string[]; max_findings?: number };
}

export function run(input: Record<string, unknown>): string | null {
  if (!existsSync(CONFIG_PATH)) return null;

  let cfg: Cfg;
  try { cfg = JSON.parse(readFileSync(CONFIG_PATH, "utf8")); } catch { return null; }
  if (cfg.enabled === false) return null;

  const toolInput = (input.tool_input ?? {}) as Record<string, unknown>;
  const filePath = typeof toolInput.file_path === "string" ? toolInput.file_path : "";
  if (!filePath || !existsSync(filePath)) return null;
  // Tool input is normalized once, then rejected unless it is inside an operator-owned root.
  const resolvedFilePath = resolve(filePath); // nosemgrep: javascript.lang.security.audit.path-traversal.path-join-resolve-traversal.path-join-resolve-traversal

  const roots = (cfg.roots ?? []).map(expand);
  if (roots.length && !roots.some((r) => isWithin(r, resolvedFilePath))) return null;

  for (const g of cfg.exclude_globs ?? []) {
    if (globToRe(expand(g)).test(resolvedFilePath) || globToRe(g).test(filePath)) return null;
  }

  const ext = extname(resolvedFilePath).toLowerCase();
  const isProse = (cfg.prose?.extensions ?? []).includes(ext);
  const isCode = (cfg.code?.extensions ?? []).includes(ext);
  if (!isProse && !isCode) return null;

  const section = isProse ? cfg.prose! : cfg.code!;
  const scanner = expand(section.scanner ?? "");
  if (!scanner || !existsSync(scanner)) return null;

  const res = spawnSync("python3", [scanner, "--json", resolvedFilePath], {
    encoding: "utf8", timeout: SCAN_TIMEOUT_MS, maxBuffer: 4 * 1024 * 1024,
  });
  if (res.error || !res.stdout?.trim()) return null;

  let data: Record<string, unknown>;
  try { data = JSON.parse(res.stdout); } catch { return null; }

  const name = resolvedFilePath.replace(HOME, "~");
  const limit = section.max_findings ?? 4;

  if (isProse) {
    const band = String((data.band as string) ?? (data.verdict as string) ?? "");
    if (!(cfg.prose?.trigger_bands ?? []).includes(band)) return null;

    const hits = Array.isArray(data.hits) ? (data.hits as Record<string, unknown>[]) : [];
    const byCat = new Map<string, number>();
    for (const h of hits) {
      const c = String(h.category ?? "");
      if (c) byCat.set(c, (byCat.get(c) ?? 0) + 1);
    }
    const top = [...byCat.entries()].sort((a, b) => b[1] - a[1]).slice(0, limit)
      .map(([c, n]) => `${c} x${n}`).join(", ");

    return [
      `✍️ PROSE GATE — ${name} scored ${data.score} [${band}].`,
      top ? `   Top: ${top}.` : "",
      `   This is a human-facing document. Fix it with the ${section.skill} skill before moving on:`,
      `   structure first (vacuity, rhythm, templates), diction last. Re-run to confirm:`,
      `   python3 ${section.scanner} "${resolvedFilePath}"`,
      `   If a flagged form is a deliberate choice, mark it <!-- human-voice: ignore <category> --> rather than leaving it unexplained.`,
    ].filter(Boolean).join("\n");
  }

  const findings = Array.isArray(data.findings) ? (data.findings as Record<string, unknown>[]) : [];
  const wanted = new Set(cfg.code?.trigger_severities ?? []);
  const hot = findings.filter((f) => wanted.has(String(f.sev ?? "")));
  if (hot.length === 0) return null;

  const lines = hot.slice(0, limit).map((f) => `   - [${f.sev}] ${f.rule}: ${f.label}`);
  return [
    `🧹 CODE GATE — ${name} has ${hot.length} high-severity AI tell${hot.length === 1 ? "" : "s"}.`,
    ...lines,
    `   Bug-class findings are wrong, not just AI-looking. Fix with the ${section.skill} skill, then re-run:`,
    `   python3 ${section.scanner} "${resolvedFilePath}"`,
  ].join("\n");
}

// Standalone entrypoint. Also composable: PostToolObserver-style hosts import run().
if (import.meta.main) {
  (async () => {
    const raw = await new Promise<string>((resolve) => {
      let d = ""; const t = setTimeout(() => resolve(d), 2000);
      process.stdin.on("data", (c) => { d += c.toString(); });
      process.stdin.on("end", () => { clearTimeout(t); resolve(d); });
      process.stdin.on("error", () => { clearTimeout(t); resolve(d); });
    });
    if (!raw.trim()) process.exit(0);
    let input: Record<string, unknown>;
    try { input = JSON.parse(raw); } catch { process.exit(0); }

    let msg: string | null = null;
    try { msg = run(input); } catch { msg = null; }

    if (msg) {
      console.log(JSON.stringify({
        hookSpecificOutput: {
          hookEventName: (input.hook_event_name as string) || "PostToolUse",
          additionalContext: msg,
        },
      }));
    }
    process.exit(0);
  })().catch(() => process.exit(0));
}
