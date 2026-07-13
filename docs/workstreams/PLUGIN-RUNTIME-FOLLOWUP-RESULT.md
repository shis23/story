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
| alias | 10 |
| derived | 3 |
| shim | 5 |
| degraded | 4 |
| intentionally_unsupported | 10 |
| noop | 4 |

Report generators:
- `frontend/src/utils/pluginCompatReport.js`
- Rust inventory: `crates/infra-plugin-host/src/compat_matrix.rs`

### Key behavior changes

1. **Terminal turn commit only for MESSAGE_* fan-out**
   Only `pipeline.committed` or payloads with explicit terminal markers (`terminalTurnCommit` / turn+attempt Final) map to `MESSAGE_RECEIVED|CHARACTER_MESSAGE_RENDERED|CHAT_CHANGED`.
   Bare `state_changed{Committed}` after `append_ai_draft` does **not** fan out, because the variant is still Draft and may be discarded.

2. **Injectable saveChat adapter**
   Host route `chat.save` + `createSaveChatAdapter`. Iframe tries host first; 500ms timeout falls back to degraded local mirror. Late host success after timeout cannot rewrite the settled promise (generation token). `await saveChat() === true` remains ST-compatible.

3. **Popup / request headers adapters**
   Host routes `ui.popup` / `ui.requestHeaders` with short timeouts so missing handlers never hang. Headers redact Authorization / X-ApiKey / Proxy-Authorization / cookie variants.

4. **Prompt-hook production wiring**
   `usePluginBridge` now passes generationId, correlationId, payload budget, and live permission resolver. `PluginHost` disables bridge-level timeout (`timeoutMs: null`) so the outer runtime owns timeout audits.

5. **Audit path**
   `recordPromptHookAudit` sanitizes, chains, and retains records in the live store. Integrity hashes are FNV-1a local corruption detection (no trusted head / not a crypto seal). No prompt bodies, secrets, or stacks.

6. **Slash**
   Unknown single commands return explicit unsupported objects; unknown pipe segments throw and stop later segments.

## Gates

### Frontend

```text
npm test
# 295 passed

npm run build
# PASS
```

### Rust

```text
CARGO_TARGET_DIR=C:\tmp\storyforge-parallel-target
cargo test -p storyforge-infra-plugin-host
# 25 passed

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
| Draft state_changed does not emit MESSAGE_* | PASS (terminal markers only) |
| Production budget/revocation/correlation wired | PASS via usePluginBridge |
| Bridge/outer double-timeout no longer masks timeout as ok | PASS (`timeoutMs: null` in PluginHost; bridge timeout rejects) |
| Header redaction covers X-ApiKey / Proxy-Authorization | PASS |
| saveChat late success after timeout ignored | PASS |
| Audit chain/query/retention on live path | PASS in recordPromptHookAudit |
| Integrity claim honesty | FNV-1a local chain only, not crypto tamper-evidence |
| Rust AuditRecord secret-safe constructor | PASS (`AuditRecord::redacted`) |

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
2. Production Accept path must emit `pipeline.committed` or terminal markers; draft state_changed alone will not notify ST message listeners.
3. saveChat default remains degraded until a real host adapter is injected outside tauri-app storage.
4. Audit integrity hashes are not cryptographic evidence.
5. Not full ST 99; release copy must stay honest.

## Merge advice

**Recommend merge after this review-fix.**

The critical Accept/Discard safety bug is fixed, production wiring for budget/revocation/correlation/audit chain is in place, timeout classification is honest, and claims no longer overstate integrity guarantees. Remaining risk is real GUI/third-party plugin hand testing, not this slice's gate health.
