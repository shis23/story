#!/usr/bin/env python3
"""Per-turn gate monitor for cot-verify matrix logs."""
from __future__ import annotations

import json
import os
import re
import sys
import time
from collections import Counter, defaultdict
from pathlib import Path


def load_jsonl(path: Path) -> list[dict]:
    if not path.exists():
        return []
    rows: list[dict] = []
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            rows.append(json.loads(line))
        except Exception:
            continue
    return rows


def parse_run_id(err_path: Path) -> str | None:
    if not err_path.exists():
        return None
    text = err_path.read_text(encoding="utf-8", errors="replace")
    match = re.search(r"run_id=(run-coverage-[0-9a-f\-]+)", text)
    return match.group(1) if match else None


def emit_plan_system_hashes(calls: list[dict]) -> Counter:
    counter: Counter = Counter()
    for row in calls:
        tools = row.get("tools_offered") or []
        if "emit_plan" in tools:
            counter[row.get("system_hash16")] += 1
    return counter


def get_character_stats(calls: list[dict]) -> tuple[int, int, float | None]:
    ok = err = 0
    for row in calls:
        for step in row.get("tool_steps") or []:
            if step.get("tool_name") != "get_character" or step.get("kind") != "result":
                continue
            detail = step.get("detail") or ""
            if step.get("ok") is False or "error_class" in detail or "tool_error" in detail:
                err += 1
            else:
                ok += 1
    total = ok + err
    return ok, err, (err / total if total else None)


def pid_running(pid_path: Path) -> bool:
    if not pid_path.exists():
        return False
    try:
        os.kill(int(pid_path.read_text(encoding="utf-8").strip()), 0)
        return True
    except Exception:
        return False


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: monitor_cot_verify.py LOGDIR")
        return 2

    logdir = Path(sys.argv[1])
    evid = logdir.parent
    report = logdir / "PER_TURN_GATES.md"
    seen_accept: dict[str, int] = defaultdict(int)
    gate_announced = False

    print(f"monitoring {logdir}", flush=True)

    while True:
        lines = [
            "# CoT verify per-turn gates",
            "",
            f"logdir: `{logdir}`",
            f"time: {time.strftime('%Y-%m-%d %H:%M:%S')}",
            "",
        ]
        hashes: dict[str, Counter] = {}
        any_alive = False

        for arm in ["disabled__native", "prompted__native"]:
            err_path = logdir / f"{arm}.err.log"
            meta_path = logdir / f"{arm}.meta.txt"
            pid_path = logdir / f"{arm}.pid"
            rid = parse_run_id(err_path)
            running = pid_running(pid_path)
            any_alive = any_alive or running
            meta_txt = (
                meta_path.read_text(encoding="utf-8", errors="replace")
                if meta_path.exists()
                else ""
            )
            finished = "exit=" in meta_txt

            lines.append(f"## {arm}")
            lines.append(f"- running={running} finished={finished} run_id=`{rid}`")
            if not rid:
                lines.append("- waiting for run_id...")
                lines.append("")
                continue

            run_dir = evid / rid
            checkpoints = load_jsonl(run_dir / "endurance_checkpoint.jsonl")
            calls = load_jsonl(run_dir / "endurance_calls.jsonl")
            turns = load_jsonl(run_dir / "endurance_turns.jsonl")
            manifest_path = run_dir / "endurance_manifest.jsonl"

            accepted = checkpoints[-1].get("accepted_turn_number") if checkpoints else 0
            calls_used = checkpoints[-1].get("calls_used") if checkpoints else 0
            identity = (checkpoints[-1].get("run_identity") if checkpoints else {}) or {}
            sys_hashes = emit_plan_system_hashes(calls)
            hashes[arm] = sys_hashes
            ok, err, rate = get_character_stats(calls)

            lines.append(
                f"- accepted={accepted}/12 calls={calls_used} turns_rows={len(turns)} call_rows={len(calls)}"
            )
            lines.append(
                f"- identity.reasoning={identity.get('reasoning_mode')} tool={identity.get('tool_mode')}"
            )
            lines.append(f"- emit_plan system_hash tops: {sys_hashes.most_common(3)}")
            rate_s = "None" if rate is None else f"{rate:.4f}"
            lines.append(f"- get_character ok/err/rate: {ok}/{err}/{rate_s}")

            if accepted and accepted > seen_accept[arm]:
                for turn in turns:
                    turn_index = turn.get("turn_index")
                    if not turn_index:
                        continue
                    if seen_accept[arm] < turn_index <= accepted:
                        msg = (
                            f"[{time.strftime('%H:%M:%S')}] {arm} ACCEPTED turn "
                            f"{turn_index}/12 text_len={turn.get('text_len')} "
                            f"q_err={turn.get('quality_error_count')} "
                            f"q_warn={turn.get('quality_warning_count')} "
                            f"autofix={turn.get('autofix_attempts')} "
                            f"force={turn.get('force_accept')} calls~{calls_used}"
                        )
                        print(msg, flush=True)
                        lines.append(
                            f"- NEW accept turn {turn_index}: text_len={turn.get('text_len')} "
                            f"quality_err={turn.get('quality_error_count')} "
                            f"warn={turn.get('quality_warning_count')} "
                            f"autofix={turn.get('autofix_attempts')} force={turn.get('force_accept')}"
                        )
                seen_accept[arm] = accepted

            err_text = (
                err_path.read_text(encoding="utf-8", errors="replace")
                if err_path.exists()
                else ""
            )
            if "SQLITE ENDURANCE coverage PASS" in err_text:
                lines.append("- stage PASS seen")
            if "FAIL CLOSED" in err_text or "seal/verify error" in err_text:
                lines.append("- failure/seal notes:")
                for line in err_text.splitlines():
                    if any(
                        key in line
                        for key in (
                            "FAIL CLOSED",
                            "plan_parse",
                            "seal/verify",
                            "os error 33",
                            "coverage PASS",
                        )
                    ):
                        lines.append(f"  - `{line.strip()[:220]}`")

            if manifest_path.exists():
                manifest = json.loads(
                    manifest_path.read_text(encoding="utf-8").splitlines()[0]
                )
                lines.append(
                    f"- stage_manifest acceptance={manifest.get('acceptance')} "
                    f"accepted={manifest.get('accepted_turns')} calls={manifest.get('calls_used')}"
                )
            lines.append("")

        lines.append("## Injection gate")
        d_hashes = hashes.get("disabled__native") or Counter()
        p_hashes = hashes.get("prompted__native") or Counter()
        d_top = d_hashes.most_common(1)
        p_top = p_hashes.most_common(1)
        if d_top and p_top and d_top[0][0] and p_top[0][0]:
            disabled_hash, prompted_hash = d_top[0][0], p_top[0][0]
            if disabled_hash == prompted_hash:
                lines.append(
                    f"- **FAIL**: emit_plan system_hash identical `{disabled_hash}` — CoT still not injected"
                )
                if not gate_announced:
                    print(
                        f"[{time.strftime('%H:%M:%S')}] GATE FAIL identical system_hash {disabled_hash}",
                        flush=True,
                    )
                    gate_announced = True
            else:
                lines.append(
                    f"- **PASS**: emit_plan system_hash differs disabled=`{disabled_hash}` prompted=`{prompted_hash}`"
                )
                if not gate_announced:
                    print(
                        f"[{time.strftime('%H:%M:%S')}] GATE PASS system_hash differs "
                        f"disabled={disabled_hash} prompted={prompted_hash}",
                        flush=True,
                    )
                    gate_announced = True
        else:
            lines.append("- waiting for emit_plan hashes on both arms")

        report.write_text("\n".join(lines) + "\n", encoding="utf-8")

        if not any_alive:
            # allow brief settle for final meta/manifest writes
            time.sleep(5)
            still = any(
                pid_running(logdir / f"{arm}.pid")
                for arm in ["disabled__native", "prompted__native"]
            )
            if not still:
                print("ALL ARMS DONE", flush=True)
                break
        time.sleep(15)

    print(f"report written {report}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
