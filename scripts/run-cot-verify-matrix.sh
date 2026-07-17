#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
EVID="${STORYFORGE_EVAL_EVIDENCE_ROOT:-/c/Users/Predator/storyforge-evidence}"
ENV_FILE="${EVID}/.eval-env.local"
if [[ ! -f "$ENV_FILE" ]]; then
  echo "missing $ENV_FILE" >&2
  exit 1
fi
# shellcheck disable=SC1090
set -a
source "$ENV_FILE"
set +a
export STORYFORGE_EVAL_REAL_LLM=1
export STORYFORGE_EVAL_EVIDENCE_ROOT="$EVID"
export STORYFORGE_EVAL_ENDURANCE_STAGE=coverage
export STORYFORGE_EVAL_SUPPLEMENTAL_MATRIX=1
export STORYFORGE_EVAL_META_PROBE=1
export STORYFORGE_EVAL_CHARACTER_EXTRACTOR_PROBE=1
export STORYFORGE_EVAL_CACHE_PROBE=1
export STORYFORGE_EVAL_MAX_TURNS=12
export STORYFORGE_EVAL_MAX_CALLS=400
export STORYFORGE_EVAL_TIMEOUT_SECS=300
export LLM_TOOL_MODE=native

if [[ -n "$(git -C "$ROOT" status --porcelain)" ]]; then
  echo "dirty worktree; real evidence requires clean git" >&2
  git -C "$ROOT" status --porcelain >&2
  exit 1
fi

TS=$(date +%Y%m%d-%H%M%S)
LOGDIR="$EVID/cot-verify-${TS}"
mkdir -p "$LOGDIR"
{
  echo "logdir=$LOGDIR"
  echo "commit=$(git -C "$ROOT" rev-parse HEAD)"
  echo "clean=yes"
  echo "model=$LLM_MODEL"
  echo "base=$LLM_BASE_URL"
} | tee "$LOGDIR/launch.txt"

PIDS=()
for r in disabled prompted; do
  name="${r}__native"
  out="$LOGDIR/${name}.out.log"
  err="$LOGDIR/${name}.err.log"
  meta="$LOGDIR/${name}.meta.txt"
  {
    echo "arm=$name"
    echo "reasoning=$r"
    echo "tool_mode=native"
    echo "started_unix=$(date +%s)"
    echo "commit=$(git -C "$ROOT" rev-parse HEAD)"
  } >"$meta"
  (
    export STORYFORGE_EVAL_REASONING_MODE="$r"
    export LLM_TOOL_MODE=native
    cd "$ROOT"
    cargo test -p harness-real-llm --test endurance_sqlite_real_llm endurance_sqlite_real_llm_staged -- --ignored --nocapture
    code=$?
    echo "exit=$code finished_unix=$(date +%s)" >>"$meta"
    exit $code
  ) >"$out" 2>"$err" &
  echo $! >"$LOGDIR/${name}.pid"
  PIDS+=("$!")
  echo "launched $name pid=$!"
  sleep 3
done
printf "%s\n" "${PIDS[@]}" >"$LOGDIR/all.pids"
echo "ALL_LAUNCHED count=${#PIDS[@]} logdir=$LOGDIR"
