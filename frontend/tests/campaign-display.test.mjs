import test from 'node:test'
import assert from 'node:assert/strict'
import {
  formatDiffValue,
  formatJsonValue,
  inferVarType,
  instanceLabel,
  knowledgeSourceText,
  parseVariableInput,
  propagationText,
  relayChainText,
  routingText,
  shortId,
  sourceLabel,
  variableDisplayName,
} from '../src/utils/campaignDisplay.js'

test('formats MVU routing labels for known and fallback shapes', () => {
  assert.equal(routingText(null), '')
  assert.equal(routingText({ Native: {} }), '原生')
  assert.equal(routingText({ kind: 'native' }), '原生')
  assert.equal(routingText({ webview_reason: 'needs DOM' }), '混合（needs DOM）')
  assert.equal(routingText({ Hybrid: { webview_reason: 'script bridge' } }), '混合（script bridge）')
  assert.equal(routingText({ kind: 'webview' }), '混合（）')
})

test('formats diff values without losing object structure', () => {
  assert.equal(formatDiffValue(null), '（无）')
  assert.equal(formatDiffValue(undefined), '（无）')
  assert.equal(formatDiffValue('plain text'), 'plain text')
  assert.equal(formatDiffValue({ hp: 12, status: 'ok' }), '{\n  "hp": 12,\n  "status": "ok"\n}')
  assert.equal(formatDiffValue(['a', 2]), '[\n  "a",\n  2\n]')
  assert.equal(formatDiffValue(false), 'false')
})

test('maps knowledge source labels and preserves unknown or empty fallback values', () => {
  assert.equal(knowledgeSourceText('witnessed'), '👁 亲眼')
  assert.equal(knowledgeSourceText('told_by_other'), '💬 被告知')
  assert.equal(knowledgeSourceText('inferred'), '🔮 推断')
  assert.equal(knowledgeSourceText('backstory'), '📖 背景')
  assert.equal(knowledgeSourceText('rumor'), 'rumor')
  assert.equal(knowledgeSourceText(''), '')
  assert.equal(knowledgeSourceText(undefined), undefined)
})

test('formats knowledge relationship labels with id fallbacks', () => {
  assert.equal(shortId('abcdefghi'), 'abcdefgh')
  assert.equal(shortId(''), '')
  assert.equal(instanceLabel({ character_name: '梁元', character_id: '123456789' }), '梁元')
  assert.equal(instanceLabel({ character_id: '123456789' }), '实例 12345678')
  assert.equal(instanceLabel({}), '未知角色')
  assert.equal(sourceLabel({ source_character_name: '旁白', source_character_id: 'abcdefghi' }), '旁白')
  assert.equal(sourceLabel({ source_character_id: 'abcdefghi' }), '实例 abcdefgh')
  assert.equal(sourceLabel({}), '')
  assert.equal(relayChainText({ relay_chain_text: 'A > B', source_knowledge_id: 'abcdefghi' }), 'A > B')
  assert.equal(relayChainText({ source_knowledge_id: 'abcdefghi' }), '上游 abcdefgh')
  assert.equal(relayChainText({}), '')
})

test('formats propagation visibility labels', () => {
  assert.equal(propagationText({ propagation: 'private' }), '🔒 封口')
  assert.equal(propagationText({ propagation: 'group:team-a' }), '限制 team-a')
  assert.equal(propagationText({ propagation: 'public' }), '')
  assert.equal(propagationText({}), '')
})

test('infers editable variable control type from JSON values', () => {
  assert.equal(inferVarType(true), 'bool')
  assert.equal(inferVarType(12), 'int')
  assert.equal(inferVarType(12.5), 'float')
  assert.equal(inferVarType(Number.NaN), 'float')
  assert.equal(inferVarType(['tag']), 'json')
  assert.equal(inferVarType({ hp: 10 }), 'json')
  assert.equal(inferVarType(null), 'string')
  assert.equal(inferVarType(undefined), 'string')
  assert.equal(inferVarType('12'), 'string')
})

test('formats JSON values for variable textareas with string fallback', () => {
  assert.equal(formatJsonValue({ hp: 10 }), '{\n  "hp": 10\n}')
  assert.equal(formatJsonValue(['a']), '[\n  "a"\n]')

  const circular = {}
  circular.self = circular
  assert.equal(formatJsonValue(circular), '[object Object]')
})

test('parses edited variable values without flattening their types', () => {
  assert.equal(parseVariableInput(false, 'bool'), false)
  assert.equal(parseVariableInput('42', 'int'), 42)
  assert.equal(parseVariableInput('3.5', 'float'), 3.5)
  assert.deepEqual(parseVariableInput('{"mood":"calm"}', 'json'), { mood: 'calm' })
  assert.equal(parseVariableInput('plain', 'string'), 'plain')
})

test('keeps invalid numeric and JSON edits as text instead of losing input', () => {
  assert.equal(parseVariableInput('not-a-number', 'int'), 'not-a-number')
  assert.equal(parseVariableInput('{broken', 'json'), '{broken')
})

test('shows Chinese names for built-in story and character variables', () => {
  assert.equal(variableDisplayName('story_clock'), '故事时间')
  assert.equal(variableDisplayName('world_state'), '世界大势')
  assert.equal(variableDisplayName('hp'), '生命值')
  assert.equal(variableDisplayName('mood'), '情绪')
  assert.equal(variableDisplayName('relationship_to_player'), '与玩家关系')
  assert.equal(variableDisplayName('danger_level'), '危险等级')
})

test('prefers the imported schema label and preserves unknown custom keys', () => {
  assert.equal(variableDisplayName('dragon_favor', '巨龙好感'), '巨龙好感')
  assert.equal(variableDisplayName('arcane_signal'), 'arcane signal')
  assert.equal(variableDisplayName('灵力'), '灵力')
})
