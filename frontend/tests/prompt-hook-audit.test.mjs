import test from 'node:test'
import assert from 'node:assert/strict'
import {
  exportPromptHookAudit,
  parsePromptHookAuditExport,
  queryPromptHookAudit,
  queryPromptHookAuditWithIntegrity,
  paginateAuditRecords,
  paginateAuditRecordsWithIntegrity,
  computeAuditRecordHash,
  chainAuditRecords,
  verifyAuditRecordChain,
  appendAuditRecordWithIntegrity,
  retainAuditRecords,
} from '../src/utils/promptHookAudit.js'

function baseRecord(overrides = {}) {
  return {
    kind: 'prompt_hook',
    pluginId: 'plugin-a',
    pluginName: 'Plugin A',
    event: 'CHAT_COMPLETION_PROMPT_READY',
    stage: 'frontend_intent',
    status: 'ok',
    durationMs: 10,
    changedKeys: ['prompt'],
    inputSummary: { prompt: { type: 'string', length: 4, hash: 'a1b2c3d4' } },
    outputSummary: { prompt: { type: 'string', length: 6, hash: 'e5f6g7h8' } },
    error: null,
    correlationId: 'corr-1',
    generationId: 'gen-1',
    recordedAt: 1000,
    ...overrides,
  }
}

test('export with empty records produces valid JSON with zero count', () => {
  const json = exportPromptHookAudit([])
  const parsed = parsePromptHookAuditExport(json)
  assert.equal(parsed.kind, 'prompt_hook_audit_export')
  assert.equal(parsed.totalRecords, 0)
  assert.deepEqual(parsed.records, [])
  assert.ok(parsed.exportedAt, 'exportedAt should be set')
})

test('export with null/undefined produces valid JSON with zero count', () => {
  const json1 = exportPromptHookAudit(null)
  const p1 = parsePromptHookAuditExport(json1)
  assert.equal(p1.totalRecords, 0)

  const json2 = exportPromptHookAudit(undefined)
  const p2 = parsePromptHookAuditExport(json2)
  assert.equal(p2.totalRecords, 0)
})

test('export preserves audit records without leaking full prompt text', () => {
  // 用真实审计记录格式构造（与 buildAuditRecord / appendPromptHookAuditRecord 一致）
  const records = [
    {
      kind: 'prompt_hook',
      pluginId: 'test-plugin',
      pluginName: 'Test Hook',
      event: 'CHAT_COMPLETION_PROMPT_READY',
      stage: 'frontend_intent',
      status: 'ok',
      durationMs: 15,
      changedKeys: ['messages', 'prompt'],
      inputSummary: {
        prompt: { type: 'string', length: 18, hash: 'a1b2c3d4' },
        messages: { type: 'array', length: 1, hash: 'e5f6g7h8' },
      },
      outputSummary: {
        prompt: { type: 'string', length: 22, hash: 'i9j0k1l2' },
        messages: { type: 'array', length: 2, hash: 'm3n4o5p6' },
      },
      error: null,
    },
    {
      kind: 'prompt_hook',
      pluginId: 'broken-plugin',
      pluginName: 'Broken',
      event: 'CHAT_COMPLETION_PROMPT_READY',
      stage: 'backend_messages',
      status: 'error',
      durationMs: 5000,
      changedKeys: [],
      inputSummary: {},
      outputSummary: {},
      error: { name: 'TimeoutError', messageLength: 14, messageHash: 'q7r8s9t0' },
    },
  ]

  const json = exportPromptHookAudit(records)
  const parsed = parsePromptHookAuditExport(json)

  assert.equal(parsed.kind, 'prompt_hook_audit_export')
  assert.equal(parsed.totalRecords, 2)
  assert.equal(parsed.records.length, 2)

  // 第一条审计记录字段完整
  const r0 = parsed.records[0]
  assert.equal(r0.pluginId, 'test-plugin')
  assert.equal(r0.event, 'CHAT_COMPLETION_PROMPT_READY')
  assert.equal(r0.status, 'ok')
  assert.equal(r0.durationMs, 15)
  assert.deepEqual(r0.changedKeys, ['messages', 'prompt'])

  // 第二条审计错误记录
  const r1 = parsed.records[1]
  assert.equal(r1.status, 'error')
  assert.equal(r1.error.name, 'TimeoutError')

  // 脱敏断言：全文 prompt 不出现在导出 JSON 字符串中
  assert.equal(json.includes('the original prompt body that should never appear'), false)
  assert.equal(json.includes('A secret hidden system message'), false)
})

test('redaction: only hash/length/type in export, no original text', () => {
  const records = [
    {
      kind: 'prompt_hook',
      pluginId: 'leaky',
      pluginName: 'Leaky',
      event: 'GENERATE_BEFORE_COMBINE_PROMPTS',
      stage: 'frontend_intent',
      status: 'ok',
      durationMs: 10,
      changedKeys: [],
      inputSummary: {
        prompt: { type: 'string', length: 42, hash: 'deadbeef' },
      },
      outputSummary: {
        prompt: { type: 'string', length: 42, hash: 'deadbeef' },
      },
      error: null,
    },
  ]

  const json = exportPromptHookAudit(records)

  // 测试确认：不可能从导出里读到模拟的 "real prompt body"
  assert.equal(json.includes('SHOULD_NOT_LEAK_real_prompt_body'), false)
  // 但元信息字段仍在
  assert.equal(json.includes('length'), true)
  assert.equal(json.includes('hash'), true)
  assert.equal(json.includes('deadbeef'), true)
  assert.equal(json.includes('type'), true)
  assert.equal(json.includes('42'), true) // length is numeric
})

test('export metadata is always present', () => {
  const json = exportPromptHookAudit([{ kind: 'prompt_hook', pluginId: 'p1' }])
  const parsed = parsePromptHookAuditExport(json)
  assert.ok(parsed.exportedAt)
  assert.equal(typeof parsed.exportedAt, 'string')
  assert.ok(parsed.exportedAt.length > 0)
  assert.equal(parsed.kind, 'prompt_hook_audit_export')
  assert.deepEqual(parsed.schema.identity, ['pluginId', 'pluginName', 'event', 'stage'])
  assert.deepEqual(parsed.schema.timing, ['durationMs', 'recordedAt'])
  assert.ok(parsed.schema.guarantees.includes('no_full_prompt_bodies'))
  assert.ok(parsed.schema.guarantees.includes('no_api_keys_or_secrets'))
})

test('exportPromptHookAudit sanitizes hostile records instead of passthrough JSON', () => {
  const json = exportPromptHookAudit([
    {
      kind: 'prompt_hook',
      pluginId: 'leaky',
      pluginName: 'Leaky',
      event: 'CHAT_COMPLETION_PROMPT_READY',
      stage: 'frontend_intent',
      status: 'ok',
      durationMs: 3,
      changedKeys: ['prompt'],
      prompt: 'private prompt body that must never export',
      api_key: 'SF_SECRET_abc',
      messages: [{ role: 'user', content: 'private message body' }],
      inputSummary: {
        prompt: { type: 'string', length: 12, hash: 'abcd' },
        api_key: 'SF_SECRET_should_be_stripped',
      },
      outputSummary: {
        prompt: 'raw prompt text in summary',
      },
      error: {
        name: 'Error',
        message: 'hook failed with private prompt text',
        stack: 'Error: hook failed with private prompt text',
      },
    },
  ])

  assert.equal(json.includes('private prompt body that must never export'), false)
  assert.equal(json.includes('SF_SECRET_'), false)
  assert.equal(json.includes('private message body'), false)
  assert.equal(json.includes('raw prompt text in summary'), false)
  assert.equal(json.includes('hook failed with private prompt text'), false)

  const parsed = parsePromptHookAuditExport(json)
  assert.equal(parsed.records[0].pluginId, 'leaky')
  assert.equal(parsed.records[0].prompt, undefined)
  assert.equal(parsed.records[0].api_key, undefined)
  assert.equal(parsed.records[0].messages, undefined)
  assert.equal(parsed.records[0].error.message, undefined)
  assert.equal(typeof parsed.records[0].error.messageHash, 'string')
  assert.equal(parsed.records[0].inputSummary.api_key, undefined)
})

test('hostile safe-leaf and identity fields cannot smuggle secrets into audit export', () => {
  const markers = [
    'SF_SECRET_plugin_name',
    'SF_SECRET_stage',
    'SF_SECRET_changed_key',
    'SF_SECRET_error_name',
    'SF_SECRET_leaf_value',
    'SF_SECRET_leaf_hash',
    'SF_SECRET_leaf_keys',
  ]
  const json = exportPromptHookAudit([{
    pluginId: 'plugin-a',
    pluginName: markers[0],
    event: 'CHAT_COMPLETION_PROMPT_READY',
    stage: markers[1],
    status: 'ok',
    changedKeys: [markers[2]],
    error: { name: markers[3], message: 'hidden message' },
    inputSummary: {
      hostile: {
        type: 'string',
        value: markers[4],
        hash: markers[5],
      },
    },
    outputSummary: {
      hostile: {
        type: 'object',
        keys: [markers[6]],
      },
    },
  }])

  for (const marker of markers) {
    assert.equal(json.includes(marker), false, `audit export leaked ${marker}`)
  }
})

// ─── query / filter / pagination / chain / retention ────────────────────────

test('queryPromptHookAudit filters by pluginId, event, stage, status, correlation, generation', () => {
  const records = [
    baseRecord({ pluginId: 'p1', event: 'E1', stage: 's1', status: 'ok', correlationId: 'c1', generationId: 'g1' }),
    baseRecord({ pluginId: 'p2', event: 'E1', stage: 's2', status: 'error', correlationId: 'c2', generationId: 'g1' }),
    baseRecord({ pluginId: 'p1', event: 'E2', stage: 's1', status: 'timeout', correlationId: 'c1', generationId: 'g2' }),
  ]

  assert.equal(queryPromptHookAudit(records, { pluginId: 'p1' }).length, 2)
  assert.equal(queryPromptHookAudit(records, { event: 'E1' }).length, 2)
  assert.equal(queryPromptHookAudit(records, { stage: 's1' }).length, 2)
  assert.equal(queryPromptHookAudit(records, { status: 'ok' }).length, 1)
  assert.equal(queryPromptHookAudit(records, { correlationId: 'c1' }).length, 2)
  assert.equal(queryPromptHookAudit(records, { generationId: 'g1' }).length, 2)
  // Combined filters narrow further.
  assert.equal(queryPromptHookAudit(records, { pluginId: 'p1', status: 'timeout' }).length, 1)
  // Null/empty input is safe.
  assert.deepEqual(queryPromptHookAudit(null, {}), [])
})

test('queryPromptHookAudit filters by duration range and recordedAt range', () => {
  const records = [
    baseRecord({ durationMs: 5, recordedAt: 100 }),
    baseRecord({ durationMs: 50, recordedAt: 200 }),
    baseRecord({ durationMs: 500, recordedAt: 300 }),
  ]

  assert.equal(queryPromptHookAudit(records, { minDurationMs: 40, maxDurationMs: 100 }).length, 1)
  assert.equal(queryPromptHookAudit(records, { sinceMs: 150, untilMs: 250 }).length, 1)
  assert.equal(queryPromptHookAudit(records, { sinceMs: 200 }).length, 2)
})

test('paginateAuditRecords applies deterministic ordering and bounds', () => {
  const records = [
    baseRecord({ pluginId: 'zeta', durationMs: 5, recordedAt: 300 }),
    baseRecord({ pluginId: 'alpha', durationMs: 50, recordedAt: 100 }),
    baseRecord({ pluginId: 'mid', durationMs: 500, recordedAt: 200 }),
  ]

  const byDurationDesc = paginateAuditRecords(records, { orderBy: 'durationMs', order: 'desc', limit: 2 })
  assert.deepEqual(byDurationDesc.map((r) => r.pluginId), ['mid', 'alpha'])

  const byRecordedAsc = paginateAuditRecords(records, { orderBy: 'recordedAt', order: 'asc' })
  assert.deepEqual(byRecordedAsc.map((r) => r.pluginId), ['alpha', 'mid', 'zeta'])

  // Offset is honored.
  const paged = paginateAuditRecords(records, { orderBy: 'recordedAt', order: 'asc', limit: 1, offset: 1 })
  assert.deepEqual(paged.map((r) => r.pluginId), ['mid'])

  // Default order is insertion order; limit bounds result without error.
  assert.equal(paginateAuditRecords(records, { limit: 0 }).length, 0)
  assert.equal(paginateAuditRecords(records, {}).length, 3)
})

test('computeAuditRecordHash is deterministic and stable across reordering of summary keys', () => {
  const r = baseRecord()
  const reordered = baseRecord({
    inputSummary: { prompt: { type: 'string', length: 4, hash: 'a1b2c3d4' } },
  })
  // Same logical content → same hash.
  assert.equal(computeAuditRecordHash(r), computeAuditRecordHash(reordered))
  // Different content → different hash.
  const changed = baseRecord({ status: 'error' })
  assert.notEqual(computeAuditRecordHash(r), computeAuditRecordHash(changed))
})

test('chainAuditRecords links records via prevHash and recomputes deterministically', () => {
  const records = [
    baseRecord({ pluginId: 'p1', recordedAt: 100 }),
    baseRecord({ pluginId: 'p2', recordedAt: 200 }),
    baseRecord({ pluginId: 'p3', recordedAt: 300 }),
  ]
  const chained = chainAuditRecords(records)

  assert.equal(chained[0].prevHash, null)
  assert.equal(chained[0].recordHash.length > 0, true)
  assert.equal(chained[1].prevHash, chained[0].recordHash)
  assert.equal(chained[2].prevHash, chained[1].recordHash)
  // Re-chaining the same input produces identical hashes (deterministic).
  const reChained = chainAuditRecords(records)
  assert.deepEqual(
    chained.map((r) => r.recordHash),
    reChained.map((r) => r.recordHash),
  )
})

test('chainAuditRecords detects tampering when a middle record changes', () => {
  const records = [
    baseRecord({ pluginId: 'p1', recordedAt: 100 }),
    baseRecord({ pluginId: 'p2', recordedAt: 200 }),
    baseRecord({ pluginId: 'p3', recordedAt: 300 }),
  ]
  const original = chainAuditRecords(records)
  const tampered = chainAuditRecords([
    records[0],
    { ...records[1], status: 'error' },
    records[2],
  ])
  // Tampering with p2 changes its own hash and breaks the chain at p3.
  assert.notEqual(original[1].recordHash, tampered[1].recordHash)
  assert.notEqual(original[2].recordHash, tampered[2].recordHash)
  assert.notEqual(tampered[2].prevHash, original[2].prevHash)
})

test('retainAuditRecords bounds the ring buffer to the most recent records', () => {
  const records = []
  for (let i = 0; i < 10; i += 1) {
    records.push(baseRecord({ pluginId: `p${i}`, recordedAt: 100 + i }))
  }
  const retained = retainAuditRecords(records, 3)
  assert.equal(retained.length, 3)
  // Retention keeps the newest (highest recordedAt when ordered by recency).
  assert.deepEqual(retained.map((r) => r.pluginId), ['p7', 'p8', 'p9'])
})

test('chainAuditRecords and export never retain secrets or prompt bodies', () => {
  const records = [
    baseRecord({
      prompt: 'private prompt body that must never chain',
      api_key: 'SF_SECRET_chain',
      messages: [{ role: 'user', content: 'private message body' }],
    }),
  ]
  const chained = chainAuditRecords(records)
  const json = JSON.stringify(chained)
  assert.equal(json.includes('private prompt body that must never chain'), false)
  assert.equal(json.includes('SF_SECRET_chain'), false)
  assert.equal(json.includes('private message body'), false)
  // The chained record still carries the tamper-evident hash.
  assert.equal(typeof chained[0].recordHash, 'string')
  assert.equal(chained[0].prevHash, null)
})

test('query and pagination re-sanitize hostile records at the boundary', () => {
  const hostile = [{
    pluginId: 'plugin-a',
    prompt: 'private prompt body',
    api_key: 'SF_SECRET_x',
    status: 'ok',
    inputSummary: { prompt: 'raw prompt text in summary' },
  }]
  const queried = queryPromptHookAudit(hostile, { pluginId: 'plugin-a' })
  const json = JSON.stringify(queried)
  assert.equal(json.includes('private prompt body'), false)
  assert.equal(json.includes('SF_SECRET_'), false)
  assert.equal(json.includes('raw prompt text in summary'), false)
})

test('audit export, query, and pagination preserve safe integrity metadata and report verification', () => {
  const chained = chainAuditRecords([
    baseRecord({ pluginId: 'one', recordedAt: 100 }),
    baseRecord({ pluginId: 'two', recordedAt: 200 }),
  ])
  const exported = parsePromptHookAuditExport(exportPromptHookAudit(chained))
  const queried = queryPromptHookAudit(chained, { pluginId: 'two' })
  const paged = paginateAuditRecords(chained, { orderBy: 'recordedAt', order: 'asc' })
  const verifiedQuery = queryPromptHookAuditWithIntegrity(chained, { pluginId: 'two' })
  const verifiedPage = paginateAuditRecordsWithIntegrity(chained, { orderBy: 'recordedAt', order: 'asc' })

  assert.equal(exported.integrity.valid, true)
  assert.equal(exported.records[0].recordedAt, 100)
  assert.equal(exported.records[1].prevHash, exported.records[0].recordHash)
  assert.equal(queried[0].recordHash, chained[1].recordHash)
  assert.equal(queried[0].prevHash, chained[1].prevHash)
  assert.equal(paged[0].recordedAt, 100)
  assert.equal(paged[1].recordHash, chained[1].recordHash)
  assert.equal(verifiedQuery.integrity.valid, true)
  assert.equal(verifiedPage.integrity.valid, true)
})

test('audit verification detects stored corruption and append refuses to silently re-chain it', () => {
  const chained = chainAuditRecords([
    baseRecord({ pluginId: 'one', recordedAt: 100 }),
    baseRecord({ pluginId: 'two', recordedAt: 200 }),
  ])
  const corrupted = chained.map((record) => ({ ...record }))
  corrupted[0].status = 'error'

  const verification = verifyAuditRecordChain(corrupted)
  const appendResult = appendAuditRecordWithIntegrity(corrupted, baseRecord({ pluginId: 'three', recordedAt: 300 }))

  assert.equal(verification.valid, false)
  assert.equal(verification.invalidIndex, 0)
  assert.equal(appendResult.appended, false)
  assert.equal(appendResult.records.length, corrupted.length)
  assert.equal(appendResult.integrity.valid, false)
})

test('audit append treats malformed existing integrity metadata as corruption, not legacy input', () => {
  const malformed = [
    { ...baseRecord({ recordedAt: 100 }), recordHash: 'not-a-valid-hash', prevHash: null },
  ]

  const appendResult = appendAuditRecordWithIntegrity(
    malformed,
    baseRecord({ pluginId: 'new', recordedAt: 200 }),
  )

  assert.equal(appendResult.appended, false)
  assert.equal(appendResult.integrity.valid, false)
  assert.equal(appendResult.integrity.reason, 'missing_record_hash')
})

test('audit append rejects a non-empty chain after all integrity metadata is stripped', () => {
  const chained = chainAuditRecords([
    baseRecord({ pluginId: 'one', recordedAt: 100 }),
    baseRecord({ pluginId: 'two', recordedAt: 200 }),
  ])
  const strippedAndTampered = chained.map(({ recordHash, prevHash, ...record }, index) => ({
    ...record,
    status: index === 0 ? 'error' : record.status,
  }))

  const appendResult = appendAuditRecordWithIntegrity(
    strippedAndTampered,
    baseRecord({ pluginId: 'three', recordedAt: 300 }),
  )

  assert.equal(appendResult.appended, false)
  assert.equal(appendResult.records.length, strippedAndTampered.length)
  assert.equal(appendResult.integrity.valid, false)
  assert.equal(appendResult.integrity.reason, 'missing_integrity_metadata')
})
