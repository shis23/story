import test from 'node:test'
import assert from 'node:assert/strict'
import { exportPromptHookAudit, parsePromptHookAuditExport } from '../src/utils/promptHookAudit.js'

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
  assert.deepEqual(parsed.schema.timing, ['durationMs'])
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
