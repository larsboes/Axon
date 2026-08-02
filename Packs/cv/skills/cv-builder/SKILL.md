---
name: cv-builder
description: Builds a job-tailored CV PDF from a single profile-tagged master CV file, via the local `cv` CLI — pick the closest profile tag (or ask), pick a language, run the build, hand back the PDF path. Use when the user wants a CV/resume for a specific job application, role type, or language. Do not use for writing or editing CV content itself (that's the master file, edited by hand) or for any content-generation/AI-tailoring flow — this skill only filters and renders what's already in the master file.
allowed-tools: Read, Bash
---

# CV Builder

Drive the CV through **`$CV`** — the source of truth; this skill is the runbook. No content
generation here: every build is a deterministic filter over the master file, never a rewrite.

**This setup is not hardcoded.** The actual CV content (name, employer, contact info) lives in
`$AXON_PERSONAL_ROOT/data/cv/master_cv.yaml`, never in this skill or in Axon. If that file is
missing, stop and tell the user to copy `capabilities/cv/master_cv.schema.yaml` there and fill
it in — do not draft CV content yourself.

## Variables

| var | value | notes |
|-----|-------|-------|
| `$CV` | `capabilities/cv/cv` | launcher, relative to `$AXON_ROOT` |
| `$MASTER` | `$AXON_PERSONAL_ROOT/data/cv/master_cv.yaml` | the one file to maintain; `$CV` refuses to run without it |
| `$PROFILE` | one of `$CV list-profiles`' output | e.g. `swe`, `quant`, `consulting`, `general` |
| `$LANG` | `en` or `de` | defaults to `en` if the user doesn't say |

## Workflow

1. `$CV list-profiles` — read the actual tags currently defined (don't assume the list from
   memory; it's just data and changes as the master file changes).
2. Match the user's ask to the closest tag:
   - A job posting/description or a stated role type ("SWE role", "consulting") → pick the
     closest existing tag from step 1's output.
   - Ambiguous, or nothing close enough → ask the user to pick from the actual list rather than
     guessing.
3. Pick `$LANG` from what the user said (German posting/company → `de`; otherwise `en` unless
   told).
4. `$CV build --profile $PROFILE --lang $LANG` → report the printed output path back to the
   user. Building every profile/lang at once: `$CV build --all` (optionally `--all --lang de`
   to restrict to one language).
5. Never edit `$MASTER` on the user's behalf mid-conversation to "tailor" wording for a specific
   posting — that recreates the AI-tailoring flow this design deliberately left out. If the
   user wants new content, tell them to add it (with the right `profiles:` tags) to the master
   file themselves.

## Gotchas

- **No matching profile tag isn't a hard stop.** `$CV build --profile <anything>` still
  produces a valid PDF — items without a `profiles` list are always included, so an unknown tag
  just yields a generic/baseline CV. Don't treat a typo'd profile name as an error; it's a
  silent quality issue instead. Always run `list-profiles` first and match against real output.
- **`$CV` requires `typst` and `yq` on `PATH`** (`brew install typst yq`) — it checks and errors
  clearly if either is missing, no need to pre-check yourself.
- **Dates aren't bilingual.** The master file's `date` fields are plain strings (e.g. "Sep.
  2025 - Present"), not `{en, de}` pairs — a German-language build still shows English month
  abbreviations by design (kept the schema simpler; per-field localization is easy to add later
  if it ever matters enough to be worth the extra upkeep).
- **Output PDFs and the master file are personal data** — they live under
  `$AXON_PERSONAL_ROOT/data/cv/`, gitignored, never in this skill or in Axon's own repo. Don't
  copy CV content into this skill file, a commit message, or anywhere else in the public repo.

## Provenance and maintenance

`$CV` → `$AXON_ROOT/capabilities/cv/cv` (bash launcher, execs `typst compile`). Re-verify on
drift:
- master file present: `test -f "$AXON_PERSONAL_ROOT/data/cv/master_cv.yaml" && echo ok`
- launcher present + executable: `test -x "$AXON_ROOT/capabilities/cv/cv" && echo ok`
- deps on PATH: `command -v typst && command -v yq`
