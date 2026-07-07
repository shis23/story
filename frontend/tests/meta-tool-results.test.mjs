import test from 'node:test'
import assert from 'node:assert/strict'
import {
  hasMetaToolResult,
  metaToolResultKind,
  worldInfoReportFromToolResult,
  cardReportFromToolResult,
  patchProposalFromToolResult,
} from '../src/utils/metaToolResults.js'

test('normalizes backend meta tool result kind values', () => {
  assert.equal(metaToolResultKind(null), '')
  assert.equal(metaToolResultKind({}), '')
  assert.equal(metaToolResultKind({ kind: 'none' }), 'none')
  assert.equal(metaToolResultKind({ kind: 'world_info_report' }), 'world_info_report')
})

test('detects visible meta tool results from backend serde shape', () => {
  assert.equal(hasMetaToolResult({}), false)
  assert.equal(hasMetaToolResult({ tool_result: { kind: 'none' } }), false)
  assert.equal(hasMetaToolResult({ tool_result: { kind: 'card_report', name: 'A' } }), true)
})

test('extracts flattened meta tool result payloads by kind', () => {
  const worldInfo = {
    kind: 'world_info_report',
    total_entries: 3,
    constant_count: 1,
    selective_count: 2,
    conflicts: [],
  }
  const card = {
    kind: 'card_report',
    name: 'Hero',
    issues: ['missing first message'],
  }
  const patch = {
    kind: 'patch_proposed',
    description: 'Fix schema',
    action_count: 2,
  }

  assert.equal(worldInfoReportFromToolResult(worldInfo), worldInfo)
  assert.equal(cardReportFromToolResult(card), card)
  assert.equal(patchProposalFromToolResult(patch), patch)
  assert.equal(worldInfoReportFromToolResult(card), null)
  assert.equal(cardReportFromToolResult(patch), null)
  assert.equal(patchProposalFromToolResult(worldInfo), null)
})

test('does not treat legacy nested meta result shapes as backend results', () => {
  const legacy = { WorldInfoReport: { total_entries: 1 } }

  assert.equal(hasMetaToolResult({ tool_result: legacy }), false)
  assert.equal(worldInfoReportFromToolResult(legacy), null)
})
