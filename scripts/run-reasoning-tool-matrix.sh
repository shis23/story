#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
EVID="${STORYFORGE_EVAL_EVIDENCE_ROOT:-/c/Users/Predator/storyforge-evidence}"
ENV_FILE="${EVID}/.eval-env.local"
if [[ ! -f "$ENV_FILE" ]]; then
  echo "missing $ENV_FILE" >&2
  exit 1
fi
set -a
# shellcheck disable=SC1090
source "$ENV_FILE"
set +a
export STORYFORGE_EVAL_REAL_LLM=1
export STORYFORGE_EVAL_EVIDENCE_ROOT="$EVID"
export STORYFORGE_EVAL_ENDURANCE_STAGE=coverage
export STORYFORGE_EVAL_SUPPLEMENTAL_MATRIX=1
export STORYFORGE_EVAL_META_PROBE=1
export STORYFORGE_EVAL_CHARACTER_EXTRACTOR_PROBE=1
export STORYFORGE_EVAL_CACHE_PROBE=1
# generous budget
export STORYFORGE_EVAL_MAX_TURNS=12
export STORYFORGE_EVAL_MAX_CALLS=400
export STORYFORGE_EVAL_TIMEOUT_SECS=300

TS=$(date +%Y%m%d-%H%M%S)
LOGDIR="$EVID/matrix-${TS}"
mkdir -p "$LOGDIR"
echo "logdir=$LOGDIR"
echo "model=${LLM_MODEL}"
echo "base=${LLM_BASE_URL}"
echo "git=$(git -C "$ROOT" rev-parse --short HEAD) clean=$( [[ -z $(git -C "$ROOT" status --porcelain) ]] && echo yes || echo no)"

REASONINGS=(disabled native prompted)
TOOLS=(native text_fallback)
PIDS=()
for r in "${REASONINGS[@]}"; do
  for t in "${TOOLS[@]}"; do
    name="${r}__${t}"
    out="$LOGDIR/${name}.out.log"
    err="$LOGDIR/${name}.err.log"
    meta="$LOGDIR/${name}.meta.txt"
    {
      echo "arm=$name"
      echo "reasoning=$r"
      echo "tool_mode=$t"
      echo "started_unix=$(date +%s)"
      echo "commit=$(git -C "$ROOT" rev-parse HEAD)"
    } >"$meta"
    (
      export STORYFORGE_EVAL_REASONING_MODE="$r"
      export LLM_TOOL_MODE="$t"
      cd "$ROOT"
      # serialise cargo test harness filter by env only; one process per arm
      cargo test -p harness-real-llm --test endurance_sqlite_real_llm endurance_sqlite_real_llm_staged -- --ignored --nocapture
      code=$?
      echo "exit=$code finished_unix=$(date +%s)" >>"$meta"
      exit $code
    ) >"$out" 2>"$err" &
    pid=$!
    echo "$pid" >"$LOGDIR/${name}.pid"
    PIDS+=("$pid")
    echo "launched $name pid=$pid"
    # slight stagger to reduce evidence root allocation races
    sleep 3
  done
done
printf "%s\n" "${PIDS[@]}" >"$LOGDIR/all.pids"
echo "ALL_LAUNCHED count=${#PIDS[@]} logdir=$LOGDIR"
