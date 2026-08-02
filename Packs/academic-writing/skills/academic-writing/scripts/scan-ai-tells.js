#!/usr/bin/env node

/*
  Deterministic scan for AI-writing tells: word-level filler (house-style.md B2/B7) and the two
  purely-mechanical rhythm patterns from references/ai-cadence-tells.md (em-dash interruption,
  semicolon splice — patterns 3-7 in that file need editorial judgment and are NOT scanned here,
  a regex can't tell a load-bearing triad from a padded one).

  Usage:
    node scan-ai-tells.js <file-or-dir> [--ext qmd,tex,md]

  Exits 1 if any match is found (so it composes in a pre-commit-style check); prints file:line +
  the matched text + which rule fired, never silently rewrites.
*/

const fs = require('fs');
const path = require('path');

const FILLER_WORDS = [
  { pattern: /\bdelve(?:s|d|ing)?\s+into\b/gi, note: 'delve into -> examine/look at' },
  { pattern: /\bdeep[\s-]dive\b/gi, note: 'deep dive -> examine' },
  { pattern: /\bleverage(?:s|d|ing)?\b/gi, note: 'leverage -> use' },
  { pattern: /\butiliz(?:e|es|ed|ing|ation)\b/gi, note: 'utilize -> use' },
  { pattern: /\bit is (?:worth noting|important to note) that\b/gi, note: 'delete the frame, keep the fact' },
  { pattern: /\b(?:seamless|robust|comprehensive|holistic)\b/gi, note: 'empty intensifier -> name the actual property' },
  { pattern: /\b(?:crucial|pivotal|vital)\b/gi, note: 'intensifier -> delete or "important" if truly load-bearing' },
  { pattern: /\bfacilitate(?:s|d)?\b/gi, note: 'facilitate -> let/help/make possible' },
  { pattern: /\bunderscor(?:e|es|ed|ing)\b/gi, note: 'underscore -> show' },
  { pattern: /\bplays? an? (?:key|vital|central) role in\b/gi, note: 'vague relation-filler -> state the actual relation' },
];

const RHYTHM_PATTERNS = [
  {
    // A dash-delimited aside inside a sentence, resuming after the closing dash — the
    // "noun — list, of, things — continues" pattern from ai-cadence-tells.md #1.
    pattern: /[a-z][^.!?—–\n]{3,80}[—–][^.!?—–\n]{3,120}[—–][^.!?\n]{3,80}[.!?]/g,
    note: 'em-dash interruption (ai-cadence-tells.md #1) — split into two sentences or use a colon',
  },
  {
    // A semicolon joining what look like two independent clauses (both sides have a verb-shaped
    // word cluster of reasonable length) — deliberately conservative to avoid false-positiving on
    // structured ID lists like "D21; D23", which are short and numeric.
    pattern: /[a-zA-Z]{4,}[^;\n]{15,};[^;\n]{15,}[a-zA-Z]{4,}[.!?]/g,
    note: 'possible semicolon splice (ai-cadence-tells.md #2) — verify it is not a structured/ID list before flagging',
  },
];

function listFilesRecursive(dir, exts) {
  const out = [];
  for (const entry of fs.readdirSync(dir)) {
    if (entry === 'node_modules' || entry === '.git' || entry === '_freeze') continue;
    // entry comes from readdir, never CLI input; skipping links keeps recursion inside dir.
    const p = path.join(dir, entry); // nosemgrep: javascript.lang.security.audit.path-traversal.path-join-resolve-traversal.path-join-resolve-traversal
    const st = fs.lstatSync(p);
    if (st.isSymbolicLink()) continue;
    if (st.isDirectory()) out.push(...listFilesRecursive(p, exts));
    else if (exts.some((e) => p.endsWith(`.${e}`))) out.push(p);
  }
  return out;
}

function lineNumberOf(content, index) {
  return content.slice(0, index).split('\n').length;
}

function scanFile(filePath) {
  const content = fs.readFileSync(filePath, 'utf8');
  const lines = content.split('\n');
  const findings = [];

  // Markdown table rows aren't prose sentences — a "|"-delimited cell's own separators (commas,
  // semicolons listing enumerated values) aren't the splice/run-on patterns this scan targets.
  const isTableRow = (lineNo) => /^\s*\|/.test(lines[lineNo - 1] || '');

  for (const { pattern, note } of FILLER_WORDS) {
    pattern.lastIndex = 0;
    let m;
    while ((m = pattern.exec(content)) !== null) {
      const line = lineNumberOf(content, m.index);
      if (!isTableRow(line)) findings.push({ line, text: m[0], note });
    }
  }
  for (const { pattern, note } of RHYTHM_PATTERNS) {
    pattern.lastIndex = 0;
    let m;
    while ((m = pattern.exec(content)) !== null) {
      const line = lineNumberOf(content, m.index);
      if (!isTableRow(line)) findings.push({ line, text: m[0].slice(0, 100), note });
    }
  }
  return findings.sort((a, b) => a.line - b.line);
}

function main() {
  const args = process.argv.slice(2);
  const target = args[0];
  if (!target) {
    console.log('Usage: node scan-ai-tells.js <file-or-dir> [--ext qmd,tex,md]');
    process.exit(0);
  }
  const extIdx = args.indexOf('--ext');
  const exts = extIdx !== -1 ? args[extIdx + 1].split(',') : ['qmd', 'tex', 'md'];

  const resolved = path.resolve(target);
  if (!fs.existsSync(resolved)) {
    console.error(`Not found: ${resolved}`);
    process.exit(1);
  }

  const files = fs.statSync(resolved).isDirectory()
    ? listFilesRecursive(resolved, exts)
    : [resolved];

  let totalFindings = 0;
  for (const f of files) {
    const findings = scanFile(f);
    if (findings.length === 0) continue;
    totalFindings += findings.length;
    console.log(`\n${path.relative(process.cwd(), f)}:`);
    for (const { line, text, note } of findings) {
      console.log(`  L${line}: "${text}" — ${note}`);
    }
  }

  console.log(`\nScanned ${files.length} file(s), ${totalFindings} finding(s).`);
  console.log('These are review heuristics, not license to rewrite — surface each with a before/after example, do not silently edit.');
  if (totalFindings > 0) process.exit(1);
}

main();
