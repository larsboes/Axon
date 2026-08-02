#!/usr/bin/env node

/*
  Citation-key check for LaTeX (.tex) and Quarto (.qmd) projects:
  - Extracts citation keys used in .tex files (\cite{}, \textcite{}, etc.) AND .qmd files
    (@citekey / [@citekey] citeproc syntax)
  - Verifies all keys exist in a given .bib file
  - Reports unused bib entries as warnings

  Usage:
    node check-citations.js <project-dir> [path/to/references.bib]

  If the .bib path is omitted, the script searches <project-dir> recursively for the first
  *.bib file it finds. Exits 1 if any citation key is missing from the bib file.

  Generalized from a third-party academic-researcher skill's scripts/check-citations.js (2026-07),
  which was hardcoded to a fixed `references/templates/` path and .tex-only — this version takes an
  arbitrary project directory and adds .qmd/@citekey support for Quarto projects.
*/

const fs = require('fs');
const path = require('path');

function listFilesRecursive(dir) {
  const out = [];
  for (const entry of fs.readdirSync(dir)) {
    if (entry === 'node_modules' || entry === '.git' || entry === '_freeze') continue;
    // entry comes from readdir, never CLI input; skipping links keeps recursion inside dir.
    const p = path.join(dir, entry); // nosemgrep: javascript.lang.security.audit.path-traversal.path-join-resolve-traversal.path-join-resolve-traversal
    const st = fs.lstatSync(p);
    if (st.isSymbolicLink()) continue;
    if (st.isDirectory()) out.push(...listFilesRecursive(p));
    else out.push(p);
  }
  return out;
}

function extractBibKeys(bibContent) {
  const keys = new Set();
  const re = /@\w+\s*\{\s*([^,\s]+)\s*,/g;
  let m;
  while ((m = re.exec(bibContent)) !== null) {
    keys.add(m[1]);
  }
  return keys;
}

function extractCiteKeysTex(texContent) {
  const keys = new Set();
  const re =
    /\\(?:textcite|parencite|autocite|cite|citet|citep|citeauthor|citeyear)\*?\s*(?:\[[^\]]*\]\s*)*\{([^}]*)\}/g;
  let m;
  while ((m = re.exec(texContent)) !== null) {
    for (const k of m[1].split(',')) {
      const key = k.trim();
      if (key) keys.add(key);
    }
  }
  return keys;
}

function extractCiteKeysQmd(qmdContent) {
  const keys = new Set();
  // Quarto/citeproc: @key, [@key], [-@key], [see @key p. 3], multiple keys "[@a; @b]".
  // No trailing '.' in the char class — citekeys don't conventionally contain periods, and
  // allowing it swallows sentence-final punctuation ("...@key." -> wrongly matches "key.").
  const re = /(?<![\w])@([A-Za-z][A-Za-z0-9_:\-]*)/g;
  let m;
  while ((m = re.exec(qmdContent)) !== null) {
    const key = m[1];
    // Heuristic: real citekeys are almost always `authorYYYY...` (contain a digit). Plain
    // "@word" prose mentions (e.g. a literal "[@keys]" placeholder in a comment) don't — skip
    // them rather than false-positive every non-citation @-mention in the document.
    if (/\d/.test(key)) keys.add(key);
  }
  return keys;
}

function findBibFile(projectDir) {
  const all = listFilesRecursive(projectDir);
  const bib = all.find((p) => p.endsWith('.bib'));
  return bib || null;
}

function rel(base, p) {
  return path.relative(base, p);
}

function main() {
  const args = process.argv.slice(2);
  const projectDir = args[0];
  if (!projectDir) {
    console.log('Usage: node check-citations.js <project-dir> [path/to/references.bib]');
    process.exit(0);
  }
  const resolvedProjectDir = path.resolve(projectDir);
  if (!fs.existsSync(resolvedProjectDir)) {
    console.error(`Project directory not found: ${resolvedProjectDir}`);
    process.exit(1);
  }

  const bibPath = args[1] ? path.resolve(args[1]) : findBibFile(resolvedProjectDir);
  if (!bibPath || !fs.existsSync(bibPath)) {
    console.error(`No .bib file found under ${resolvedProjectDir} (or the given path doesn't exist). Pass it explicitly as the second argument.`);
    process.exit(1);
  }

  const bibContent = fs.readFileSync(bibPath, 'utf8');
  const bibKeys = extractBibKeys(bibContent);

  const allFiles = listFilesRecursive(resolvedProjectDir);
  const texFiles = allFiles.filter((p) => p.endsWith('.tex'));
  const qmdFiles = allFiles.filter((p) => p.endsWith('.qmd'));

  let ok = true;
  const allCiteKeys = new Set();

  for (const f of texFiles) {
    const content = fs.readFileSync(f, 'utf8');
    const citeKeys = extractCiteKeysTex(content);
    for (const k of citeKeys) allCiteKeys.add(k);
    const missing = [...citeKeys].filter((k) => !bibKeys.has(k)).sort();
    if (missing.length > 0) {
      ok = false;
      console.error(`Missing keys in ${rel(resolvedProjectDir, f)}: ${missing.join(', ')}`);
    }
  }

  for (const f of qmdFiles) {
    const content = fs.readFileSync(f, 'utf8');
    const citeKeys = extractCiteKeysQmd(content);
    for (const k of citeKeys) allCiteKeys.add(k);
    const missing = [...citeKeys].filter((k) => !bibKeys.has(k)).sort();
    if (missing.length > 0) {
      ok = false;
      console.error(`Missing keys in ${rel(resolvedProjectDir, f)}: ${missing.join(', ')}`);
    }
  }

  const unusedKeys = [...bibKeys].filter((k) => !allCiteKeys.has(k)).sort();
  if (unusedKeys.length > 0) {
    console.warn(`Warning: unused bib entries in ${rel(resolvedProjectDir, bibPath)}: ${unusedKeys.join(', ')}`);
  }

  if (!ok) process.exit(1);

  console.log(
    `Checked ${texFiles.length} .tex + ${qmdFiles.length} .qmd files, ${allCiteKeys.size} citation keys, ${bibKeys.size} bib entries (${unusedKeys.length} unused)`
  );
}

main();
