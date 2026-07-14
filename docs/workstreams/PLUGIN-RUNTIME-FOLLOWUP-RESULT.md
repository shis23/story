# Plugin Runtime Compatibility Follow-Up Result

- Branch: `codex/plugin-runtime-followup`
- Worktree: `C:\tmp\storyforge-plugin-runtime`
- Base plan commit: `43799c5`
- Date: 2026-07-13
- HEAD: see commit list below

## Conclusion

Completed the plugin-runtime follow-up vertical slice with TDD, then applied a security/compatibility self-audit fix pass for production wiring.

Did **not** claim full ST 99, real iframe/GUI acceptance, edit `tauri-app` / SQLite / storage code, push, or call real LLMs.

## Commits (since `43799c5`)

| Commit | Summary |
|--------|---------|
| `035ac07` | injectable persistence adapter for saveChat/popup/requestHeaders |
| `2dfba2a` | prompt-hook budgets, unload, revocation, fail policy |
| `879a82a` | audit query/filter/pagination/retention + integrity chain |
| `e8ede2b` | committed alias + slash/correlation hardening |
| `e3e3a38` | Rust correlation/permission + audit inventory |
| `e122b1c` | machine-readable + Markdown compatibility report |
| `510052b` | PLUGIN-RUNTIME-FOLLOWUP-RESULT (initial) |
| *(tip)* | review-fix: terminal-commit only, production wiring, honest integrity claims |

## Compatibility matrix

Executable frontend matrix totals after follow-up:

| Status | Count |
|--------|------:|
| total | 88 |
| implemented | 52 |
| alias | 9 |
| derived | 3 |
| shim | 5 |
| degraded | 4 |
| intentionally_unsupported | 11 |
| noop | 4 |

Report generators:
- `frontend/src/utils/pluginCompatReport.js`
- Rust inventory: `crates/infra-plugin-host/src/compat_matrix.rs`

### Key behavior changes

1. **Terminal turn commit only for MESSAGE_* fan-out**
   A Draft completion emits only `GENERATION_ENDED` with `draft_ready`; it never emits `MESSAGE_RECEIVED`.
   `handleAcceptVariant` is the frontend terminal source after the backend Accept succeeds. It emits exactly one `MESSAGE_RECEIVED` carrying `terminalTurnCommit`, terminal Turn/Attempt/Variant status, and force-accept/Degraded state; the host derives render/chat events from that one source. Bare `state_changed{Committed}` after `append_ai_draft` still does **not** fan out.

2. **Injectable saveChat adapter**
   Host route `chat.save` + `createSaveChatAdapter`. The host deduplicates same-snapshot in-flight and successful retries before the real persistence adapter runs. A 500ms iframe timeout resolves ST-compatibly but reports `persist_outcome_unknown` / `outcomeUnknown=true`; it does not claim a known local-only outcome. `await saveChat() === true` remains ST-compatible.

3. **Popup / request headers adapters**
   Host routes `ui.popup` / `ui.requestHeaders` with short timeouts so missing handlers never hang. Headers redact Authorization / X-ApiKey / Proxy-Authorization / cookie variants.

4. **Prompt-hook production wiring**
   `usePluginBridge` now passes generationId, correlationId, payload budget, and live permission resolver. `PluginHost` disables bridge-level timeout (`timeoutMs: null`) so the outer runtime owns timeout audits.

5. **Audit path**
   `recordPromptHookAudit` sanitizes and verifies the stored segment before appending. A corrupt chain is not silently re-chained. Export/query/pagination preserve safe timestamp/hash metadata and expose a verification verdict. FNV-1a is only a local accidental-corruption checksum (no trusted head, not cryptographic tamper evidence). No prompt bodies, secrets, or stacks. Rust `AuditRecord` fields are private and the type intentionally does not implement `Deserialize`.

6. **Slash**
   Unknown single commands return explicit unsupported objects; unknown pipe segments throw and stop later segments.

## Gates

### Frontend

```text
npm test
# 305 passed

npm run build
# PASS
```

### Rust

```text
CARGO_TARGET_DIR=C:\tmp\storyforge-parallel-target
cargo test -p storyforge-infra-plugin-host
# 27 passed

cargo clippy -p storyforge-infra-plugin-host --all-targets -- -D warnings
# PASS
```

### Diff check

```text
git diff --check
# PASS on current tree after whitespace cleanup
```

### Not run / prohibited

- full workspace cargo test
- real GUI / third-party iframe manual acceptance
- paid/real LLM
- no edits to tauri-app / SQLite storage / HANDOFF / RELEASE-CHECKLIST
- no push / rebase / force-push / main edits

## Security and compatibility self-audit (post-fix)

| Check | Result |
|------|--------|
| Draft write/Discard does not emit MESSAGE_RECEIVED | PASS (production composable path) |
| Accept / force Accept emits one terminal MESSAGE_RECEIVED | PASS (production composable path) |
| Production budget/revocation/correlation wired | PASS via usePluginBridge |
| Bridge/outer double-timeout no longer masks timeout as ok | PASS (`timeoutMs: null` in PluginHost; bridge timeout rejects) |
| Header redaction covers X-ApiKey / Proxy-Authorization | PASS |
| saveChat timeout/retry cannot double-write adapter | PASS (host snapshot idempotency) |
| Audit chain/query/page on live path | PASS (verify before append; no silent re-chain) |
| Integrity claim honesty | FNV-1a local accidental-corruption checksum only, not crypto tamper evidence |
| Rust AuditRecord construction/deserialization boundary | PASS (private fields + `AuditRecord::redacted`; no `Deserialize`) |

## Remaining degraded / unsupported / noop

### Degraded (4)

- `th:saveChat` default local mirror (injectable host adapter available)
- `th:callGenericPopup` no UI / default-or-null
- `th:getRequestHeaders` static/redacted JSON content-type
- `host:mock_ui` labelled mock only

### Intentionally unsupported (representative)

- after-combine / force worldinfo / tool calls / groups
- unknown slash
- bare pipeline `state_changed{Committed}` message fan-out

### Noop

- worldinfo settings/update, settings loaded, extensions first load

## Risks

1. Real third-party iframe + Tauri IPC still only mock/Node deterministic coverage.
2. Frontend terminal fan-out is covered, but real third-party iframe side effects still need manual GUI acceptance.
3. saveChat default remains degraded until a real host adapter is injected outside tauri-app storage.
4. Audit integrity hashes are not cryptographic evidence.
5. Not full ST 99; release copy must stay honest.

## Merge advice

**Recommend merge after this review-fix.**

The critical Accept/Discard safety bug is fixed, production wiring for budget/revocation/correlation/audit chain is in place, saveChat persistence retry behavior is bounded and honest, and claims no longer overstate integrity guarantees. Remaining risk is real GUI/third-party plugin hand testing, not this slice's gate health.
