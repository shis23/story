import test from 'node:test'
import assert from 'node:assert/strict'
import { assistantRoleLabel } from '../src/utils/roleLabel.js'

test('campaign 模式用 campaignName', () => {
  assert.equal(assistantRoleLabel('campaign', '寒渊谜塔', '艾莉娅'), '寒渊谜塔')
})

test('legacy 模式用 charName（忽略 campaignName）', () => {
  assert.equal(assistantRoleLabel('legacy', '寒渊谜塔', '艾莉娅'), '艾莉娅')
  assert.equal(assistantRoleLabel('legacy', undefined, '艾莉娅'), '艾莉娅')
})

test('campaign 模式 campaignName 为空时兜底 AI', () => {
  assert.equal(assistantRoleLabel('campaign', '', '艾莉娅'), 'AI')
  assert.equal(assistantRoleLabel('campaign', null, '艾莉娅'), 'AI')
  assert.equal(assistantRoleLabel('campaign', undefined, '艾莉娅'), 'AI')
})

test('legacy 模式 charName 为空时兜底 AI', () => {
  assert.equal(assistantRoleLabel('legacy', '寒渊谜塔', ''), 'AI')
  assert.equal(assistantRoleLabel('legacy', '寒渊谜塔', null), 'AI')
  assert.equal(assistantRoleLabel('legacy', '寒渊谜塔', undefined), 'AI')
})

test('none 模式始终兜底 AI（忽略所有名）', () => {
  assert.equal(assistantRoleLabel('none', '寒渊谜塔', '艾莉娅'), 'AI')
  assert.equal(assistantRoleLabel('none', '', ''), 'AI')
})

test('合并 App.vue:86-90 streamingRoleLabel 与 314-317 getAssistantRoleLabel 行为一致', () => {
  // App.vue:86-90 streamingRoleLabel: campaign ? campaign.name||'AI' : char.name||'AI'
  // App.vue:314-317 getAssistantRoleLabel: campaign ? campaign.name||'AI' : char.name||'AI'
  // 两者等价，合并后 assistantRoleLabel 应同时满足：
  assert.equal(assistantRoleLabel('campaign', '寒渊谜塔', null), '寒渊谜塔')
  assert.equal(assistantRoleLabel('campaign', null, '艾莉娅'), 'AI')
  assert.equal(assistantRoleLabel('legacy', null, '艾莉娅'), '艾莉娅')
  assert.equal(assistantRoleLabel('legacy', '寒渊谜塔', null), 'AI')
})
