#!/bin/bash
# graphify.sh — rebuild the local code-dependency graph via graphify (upstreams.toml
# [graphify]). Output lands in graphify-out/ (git-ignored -- see
# the header above for why: graphify's
# node IDs are slugified from the absolute scan path, so its output is inherently
# machine-specific and not something ARCHITECTURE.md embeds directly).
#
# oMLX is the local default. Its API key is read at call time from the same
# ~/.omlx/settings.json file the server owns; it is exported only to graphify's child process
# and never printed. tools/setup-secret.sh does NOT apply to the optional NVIDIA key: that
# script only provisions capabilities/<name>/service.toml-backed secrets, and this is a tools/
# script, not a capability. The cloud key still goes through Vaultwarden (README.md#secrets).
#
# Backend selection (semantic extraction of docs, on top of the always-free AST pass):
# see tools/graphify.env.example. Local-first: defaults to oMLX, falls back to the
# original code-only behavior if the configured backend isn't actually reachable --
# this script never hard-fails just because a backend isn't running.
#
# Usage: tools/graphify.sh
# Check only: tools/graphify.sh --check
# Bazel: bazel run //:graphify
set -e

if [ -n "${BUILD_WORKSPACE_DIRECTORY:-}" ]; then
  _lib="$BUILD_WORKSPACE_DIRECTORY/tools/lib"
else
  _lib="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/lib" && pwd)"
fi
source "$_lib/paths.sh"

cd "$AXON_ROOT"

_cfg="$AXON_PERSONAL_ROOT/config/graphify.env"
if [ -f "$_cfg" ]; then
  source "$_cfg"
fi
GRAPHIFY_BACKEND="${GRAPHIFY_BACKEND:-omlx}"

# oMLX/NVIDIA have no native graphify --backend of their own. Both speak the OpenAI chat
# completions wire format, so both go through graphify's real `openai` backend. `mlx` remains
# an alias for older private configs; new configs say what actually provides the endpoint.
_real_backend="$GRAPHIFY_BACKEND"
case "$GRAPHIFY_BACKEND" in
  omlx|mlx)
    _real_backend="openai"
    _omlx_settings="${OMLX_SETTINGS_PATH:-$HOME/.omlx/settings.json}"
    _omlx_base_url="${OMLX_BASE_URL:-http://127.0.0.1:8000/v1}"
    _omlx_model="${OMLX_MODEL:-${GRAPHIFY_MODEL:-gemma-4-26b-a4b-it-4bit}}"
    _omlx_api_key="${OMLX_API_KEY:-}"
    if [ -z "$_omlx_api_key" ] && [ -f "$_omlx_settings" ]; then
      _omlx_api_key="$(jq -er '.auth.api_key | select(type == "string" and length > 0)' "$_omlx_settings" 2>/dev/null || true)"
    fi
    # Do not reuse ambient cloud OPENAI_* values for a local server. Only the
    # OMLX_* namespace or oMLX's own settings file may populate this child.
    OPENAI_BASE_URL="$_omlx_base_url"
    OPENAI_MODEL="$_omlx_model"
    OPENAI_API_KEY="$_omlx_api_key"
    GRAPHIFY_MODEL="$_omlx_model"
    export OPENAI_BASE_URL OPENAI_MODEL OPENAI_API_KEY
    ;;
  nvidia)
    _real_backend="openai"
    : "${OPENAI_BASE_URL:=https://integrate.api.nvidia.com/v1}"
    export OPENAI_BASE_URL
    ;;
esac

_use_backend=1
case "$GRAPHIFY_BACKEND" in
  none)
    _use_backend=0
    ;;
  ollama)
    if ! curl -sf --max-time 2 http://localhost:11434 >/dev/null 2>&1; then
      echo "graphify.sh: GRAPHIFY_BACKEND=ollama but no ollama server reachable at localhost:11434" >&2
      echo "  (start it with 'ollama serve' + 'ollama pull <model>') -- falling back to code-only." >&2
      _use_backend=0
    fi
    ;;
  omlx|mlx)
    if [ -z "${OPENAI_BASE_URL:-}" ] || [ -z "${OPENAI_API_KEY:-}" ]; then
      echo "graphify.sh: GRAPHIFY_BACKEND=$GRAPHIFY_BACKEND needs OPENAI_BASE_URL + OPENAI_API_KEY" >&2
      echo "  expected oMLX settings at ${_omlx_settings:-<unset>} (see tools/graphify.env.example) -- falling back to code-only." >&2
      _use_backend=0
    elif ! printf 'Authorization: Bearer %s\n' "$OPENAI_API_KEY" |
      curl -sf --max-time 3 -H @- "${OPENAI_BASE_URL%/}/models" >/dev/null 2>&1; then
      echo "graphify.sh: oMLX is not reachable at ${OPENAI_BASE_URL%/}/models -- falling back to code-only." >&2
      _use_backend=0
    fi
    ;;
  nvidia)
    if [ -z "${OPENAI_BASE_URL:-}" ] || [ -z "${OPENAI_API_KEY:-}" ]; then
      echo "graphify.sh: GRAPHIFY_BACKEND=nvidia needs OPENAI_BASE_URL + OPENAI_API_KEY" >&2
      echo "  set in $_cfg (see tools/graphify.env.example) -- falling back to code-only." >&2
      _use_backend=0
    fi
    ;;
  *)
    echo "graphify.sh: unknown GRAPHIFY_BACKEND '$GRAPHIFY_BACKEND' -- falling back to code-only." >&2
    _use_backend=0
    ;;
esac

if [ "${1:-}" = "--check" ]; then
  if [ "$_use_backend" = "1" ]; then
    echo "graphify.sh: backend=$GRAPHIFY_BACKEND wire=$_real_backend model=${GRAPHIFY_MODEL:-${OPENAI_MODEL:-<provider-default>}} reachable"
    exit 0
  fi
  echo "graphify.sh: semantic backend unavailable; normal runs would use code-only mode" >&2
  exit 1
fi

# Pinned via `uv tool run`, deliberately NOT the bare `graphify` on PATH -- see
# upstreams.toml [graphify] for why that distinction matters here.
echo "--- graphify: rebuilding code-dependency graph ---"
if [ "$_use_backend" = "1" ]; then
  echo "--- graphify: semantic extraction via backend '$GRAPHIFY_BACKEND' ---"
  _model_args=()
  [ -n "${GRAPHIFY_MODEL:-}" ] && _model_args=(--model "$GRAPHIFY_MODEL")
  # graphify defaults to four semantic chunks in flight, and its own --help says to set 1
  # for local LLMs. A local server answers one request at a time anyway, so four in flight
  # buys no throughput and does risk the memory guard on a machine also running the model.
  # Not a fix for the failure observed here (see the note in tools/graphify.env.example):
  # that one is a parse failure, and it reproduces at concurrency 1. This is upstream's
  # documented setting for this deployment shape, correct on its own terms.
  _tuning_args=()
  case "$GRAPHIFY_BACKEND" in
    omlx|mlx|ollama) _tuning_args=(--max-concurrency 1) ;;
  esac
  uv tool run --from "graphifyy[openai]==0.9.19" graphify extract . --backend "$_real_backend" "${_model_args[@]}" "${_tuning_args[@]}"
else
  # `update`, not `extract --code-only`: the latter has a confirmed-broken --code-only
  # flag on every graphifyy release through 0.9.10 (silently demands an LLM key for doc
  # files anyway); `update` is the older, established command and has always been
  # code-only by design.
  uv tool run --from graphifyy==0.9.19 graphify update .
fi
# What actually came out, read off the artifact rather than scraped from graphify's log.
# graphify preserves a usable AST graph and exits 0 even when semantic extraction fails, so
# exit status alone cannot tell the two apart.
#
# `_origin` is the signal: graphify stamps every node and edge with the pass that produced
# it, and an AST-only graph carries `ast` on all of them. Counting anything-but-ast is
# deliberately not a match against a specific label -- upstream may name the semantic pass
# whatever it likes, and asking "did anything other than the AST pass contribute" survives
# that. Two fields that look like they would work and do not: `summary` does not exist in
# this schema at all, so any check against it reports code-only forever; `community_name`
# is populated on a pure AST run too (it is slugified from paths and symbols).
report_outcome() {
  local graph="$AXON_ROOT/graphify-out/graph.json" total=0 semantic=0 hyper=0

  if [ ! -f "$graph" ]; then
    echo "--- graphify: unavailable -- no graph.json was produced ---" >&2
    return 1
  fi
  total="$(jq -r '.nodes | length' "$graph" 2>/dev/null || echo 0)"
  semantic="$(jq -r '[.nodes[], .links[]? | select(._origin != "ast")] | length' "$graph" 2>/dev/null || echo 0)"
  hyper="$(jq -r '.hyperedges | length' "$graph" 2>/dev/null || echo 0)"

  if [ "$_use_backend" != "1" ]; then
    echo "--- graphify: code-only -- $total nodes, no semantic pass requested ---"
  elif [ "$semantic" -eq 0 ] && [ "$hyper" -eq 0 ]; then
    echo "--- graphify: code-only -- $total nodes, semantic pass via '$GRAPHIFY_BACKEND' contributed nothing ---" >&2
  else
    echo "--- graphify: semantic -- $total nodes, $semantic non-AST element(s), $hyper hyperedge(s) via '$GRAPHIFY_BACKEND' ---"
  fi
  echo "    see graphify-out/graph.html, GRAPH_REPORT.md"
}

report_outcome
