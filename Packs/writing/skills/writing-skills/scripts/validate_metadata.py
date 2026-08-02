#!/usr/bin/env python3
"""Validate a skill's SKILL.md metadata for discoverability + spec compliance.

Combines the agentskills.io rules with Anthropic's requirements and the local
directory-name convention. Prints SUCCESS to stdout, or one line per problem to
stderr (exit 1) so an agent can self-correct field-by-field.

Usage:
  validate_metadata.py --name NAME --description DESC [--dir PATH]
  validate_metadata.py --file path/to/SKILL.md          # parse frontmatter instead
"""
import re, sys, os, argparse

RESERVED = {"anthropic", "claude"}
FIRST_SECOND_PERSON = {"i", "me", "my", "we", "our", "you", "your"}
BODY_MAX_LINES, BODY_MAX_TOKENS, WARN_AT_PCT = 500, 5000, 80

def parse_frontmatter(path):
    text = open(path, encoding="utf-8").read()
    m = re.match(r"^---\s*\n(.*?)\n---", text, re.DOTALL)
    if not m:
        print("FILE ERROR: no YAML frontmatter (--- ... ---) at top of SKILL.md", file=sys.stderr)
        sys.exit(1)
    body = m.group(1)
    lines = body.split("\n")
    def field(key):
        prefix = f"{key}:"
        for i, line in enumerate(lines):
            if not line.startswith(prefix):
                continue
            rest = line[len(prefix):].strip()
            if rest in (">-", ">", "|-", "|"):
                # YAML block scalar: fold every following indented line until dedent/EOF.
                folded = []
                for cont in lines[i + 1:]:
                    if cont.strip() == "" or cont.startswith((" ", "\t")):
                        if cont.strip():
                            folded.append(cont.strip())
                        continue
                    break
                return " ".join(folded)
            return rest.strip('"').strip("'")
        return None
    return field("name"), field("description")

def check_body(path):
    """Level-2 budget. The body is read in full every time the skill triggers, so
    anything not consulted during an invocation belongs in references/ instead."""
    text = open(path, encoding="utf-8").read()
    body = re.sub(r"^---\s*\n.*?\n---\s*\n", "", text, count=1, flags=re.DOTALL)
    lines, tokens = len(body.strip().splitlines()), len(body) // 4  # chars/4: the pessimistic estimate
    errors = []
    for got, cap, unit in ((lines, BODY_MAX_LINES, "lines"), (tokens, BODY_MAX_TOKENS, "~tokens")):
        pct = got * 100 // cap
        if got > cap:
            errors.append(f"BODY ERROR: {got} {unit} is {pct}% of the {cap} budget; move bulk to references/.")
        elif pct >= WARN_AT_PCT:
            errors.append(f"BODY WARNING: {got} {unit} is {pct}% of the {cap} budget.")
    summary = (f"Body: {lines} lines ({lines * 100 // BODY_MAX_LINES}%), "
               f"~{tokens} tokens ({tokens * 100 // BODY_MAX_TOKENS}%).")
    return errors, summary

def validate(name, description, dirpath=None, extra=None, summary=""):
    errors = []
    name = name or ""
    description = description or ""

    if not (1 <= len(name) <= 64):
        errors.append(f"NAME ERROR: '{name}' is {len(name)} chars; must be 1-64.")
    if not re.match(r"^[a-z0-9]+(-[a-z0-9]+)*$", name):
        errors.append(f"NAME ERROR: '{name}' invalid; use lowercase/digits/single-hyphens, "
                      "no leading/trailing/double hyphens.")
    for w in RESERVED:
        if w in name.lower():
            errors.append(f"NAME ERROR: '{name}' contains reserved word '{w}'.")

    if dirpath:
        base = os.path.basename(os.path.normpath(dirpath))
        if base != name:
            errors.append(f"DIR ERROR: directory '{base}' must exactly match name '{name}'.")

    if not description:
        errors.append("DESCRIPTION ERROR: description is empty.")
    if len(description) > 1024:
        errors.append(f"DESCRIPTION ERROR: {len(description)} chars; must be <= 1024.")
    if "<" in description and ">" in description:
        errors.append("DESCRIPTION ERROR: must not contain XML/HTML tags.")

    # Strip quoted spans first -- a description legitimately quotes example user
    # phrases ('says it "sounds like ChatGPT"'), and those aren't the author's
    # own first/second-person narration.
    unquoted = re.sub(r'"[^"]*"', " ", description)
    words = set(re.findall(r"\b\w+\b", unquoted.lower()))
    hit = FIRST_SECOND_PERSON & words
    if hit:
        errors.append(f"STYLE ERROR: description uses first/second person {sorted(hit)}; "
                      "write third person (e.g. 'Extracts...', 'Use when...').")
    low = description.lower()
    if "use when" not in low and "use for" not in low:
        errors.append("DISCOVERY WARNING: description has no 'Use when...' trigger clause.")
    if "don't use" not in low and "do not use" not in low:
        errors.append("DISCOVERY WARNING: description has no negative trigger ('Do not use for...').")

    errors.extend(extra or [])
    hard = [e for e in errors if "WARNING" not in e]
    if summary:
        print(summary)
    if errors:
        print("\n".join(errors), file=sys.stderr)
        sys.exit(1 if hard else 0)
    print("SUCCESS: metadata is valid and optimized for discovery.")
    sys.exit(0)

if __name__ == "__main__":
    p = argparse.ArgumentParser()
    p.add_argument("--name"); p.add_argument("--description", default="")
    p.add_argument("--dir"); p.add_argument("--file")
    a = p.parse_args()
    if a.file:
        n, d = parse_frontmatter(a.file)
        body_errors, summary = check_body(a.file)
        validate(n, d, a.dir or os.path.dirname(os.path.abspath(a.file)), body_errors, summary)
    elif a.name:
        validate(a.name, a.description, a.dir)
    else:
        p.error("provide --name (and --description) or --file SKILL.md")
