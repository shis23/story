import test from 'node:test'
import assert from 'node:assert/strict'
import { createPinia, setActivePinia } from 'pinia'
import { useWritingStore } from '../../src/stores/writing.js'
import { useCampaignStore } from '../../src/stores/campaign.js'

function setup() {
  setActivePinia(createPinia())
  return useWritingStore()
}

test('generation mode is remembered independently for each campaign', () => {
  setActivePinia(createPinia())
  const writing = useWritingStore()
  const campaign = useCampaignStore()
  campaign.activeCampaign = { id: 'campaign-a', name: 'A' }

  assert.equal(writing.generationMode, 'continuation')
  writing.setGenerationMode('sequential_crew')
  assert.equal(writing.generationMode, 'sequential_crew')

  campaign.activeCampaign = { id: 'campaign-b', name: 'B' }
  assert.equal(writing.generationMode, 'continuation')
  writing.setGenerationMode('duet')
  assert.equal(writing.generationMode, 'duet')

  campaign.activeCampaign = { id: 'campaign-a', name: 'A' }
  assert.equal(writing.generationMode, 'sequential_crew')
})

test('generation mode rejects unknown wire values', () => {
  setActivePinia(createPinia())
  const writing = useWritingStore()
  const campaign = useCampaignStore()
  campaign.activeCampaign = { id: 'campaign-a' }

  writing.setGenerationMode('made_up_mode')

  assert.equal(writing.generationMode, 'continuation')
})

test('generation mode memory survives store recreation but remains keyed by campaign', () => {
  const values = new Map()
  globalThis.localStorage = {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
  }
  try {
    setActivePinia(createPinia())
    let campaign = useCampaignStore()
    let writing = useWritingStore()
    campaign.activeCampaign = { id: 'campaign-persisted' }
    writing.setGenerationMode('sequential_crew')

    setActivePinia(createPinia())
    campaign = useCampaignStore()
    writing = useWritingStore()
    campaign.activeCampaign = { id: 'campaign-persisted' }
    assert.equal(writing.generationMode, 'sequential_crew')
    campaign.activeCampaign = { id: 'campaign-new' }
    assert.equal(writing.generationMode, 'continuation')
  } finally {
    delete globalThis.localStorage
  }
})

test('turn receipt defaults every candidate to selected and can toggle by mutation index', () => {
  const writing = setup()
  writing.openTurnReceipt('node-1', {
    turn_id: 'turn-1',
    derivation_failed: false,
    items: [
      { mutation_index: 2, kind: 'chronicle', title: '纪要', detail: 'A', selected_by_default: true },
      { mutation_index: 4, kind: 'variable', title: '变量', detail: 'B', selected_by_default: true },
    ],
  })

  assert.equal(writing.pendingReceipt.nodeId, 'node-1')
  assert.deepEqual(writing.selectedReceiptMutationIndices, [2, 4])

  writing.setReceiptItemSelected(4, false)
  assert.deepEqual(writing.selectedReceiptMutationIndices, [2])
  writing.clearTurnReceipt()
  assert.equal(writing.pendingReceipt, null)
})

test('writing store 初始状态', () => {
  const s = setup()
  assert.deepEqual(s.messages, [])
  assert.equal(s.isWriting, false)
  assert.equal(s.showPipeline, false)
  assert.equal(s.activeConnection, null)
  assert.equal(s.selectedGreetingIndex, 0)
})

test('pipeline 初始结构符合冻结契约', () => {
  const s = setup()
  assert.equal(s.pipeline.state, 'idle')
  assert.equal(s.pipeline.stateLabel, '')
  assert.equal(s.pipeline.director.status, 'idle')
  assert.equal(s.pipeline.director.detail, '')
  assert.equal(s.pipeline.director.output, '')
  assert.deepEqual(s.pipeline.subagents, [])
  assert.equal(s.pipeline.editor.status, 'idle')
  assert.equal(s.pipeline.postprocess.status, 'idle')
  assert.equal(s.pipeline.postprocess.knowledge, 0)
  assert.equal(s.pipeline.postprocess.variable, 0)
  assert.equal(s.pipeline.postprocess.task, 0)
  assert.equal(s.pipeline.postprocess.reason, '')
  assert.equal(s.pipeline.quality, null)
})

test('writingMode 无 campaign 无 char 时为 none', () => {
  const s = setup()
  assert.equal(s.writingMode, 'none')
})

test('writingMode 有 activeChar 时为 legacy', () => {
  setActivePinia(createPinia())
  const writing = useWritingStore()
  const campaign = useCampaignStore()
  campaign.activeChar = { id: 'char_1', name: '艾莉丝' }
  assert.equal(writing.writingMode, 'legacy')
})

test('writingMode 有 activeCampaign 时为 campaign(优先级高于 char)', () => {
  setActivePinia(createPinia())
  const writing = useWritingStore()
  const campaign = useCampaignStore()
  campaign.activeChar = { id: 'char_1', name: '艾莉丝' }
  campaign.activeCampaign = { id: 'c1', name: '寒渊谜塔' }
  assert.equal(writing.writingMode, 'campaign')
})

test('streamingRoleLabel campaign 模式用 campaign 名', () => {
  setActivePinia(createPinia())
  const writing = useWritingStore()
  const campaign = useCampaignStore()
  campaign.activeCampaign = { id: 'c1', name: '寒渊谜塔' }
  assert.equal(writing.streamingRoleLabel, '寒渊谜塔')
})

test('streamingRoleLabel legacy 模式用 char 名', () => {
  setActivePinia(createPinia())
  const writing = useWritingStore()
  const campaign = useCampaignStore()
  campaign.activeChar = { id: 'char_1', name: '艾莉丝' }
  assert.equal(writing.streamingRoleLabel, '艾莉丝')
})

test('streamingRoleLabel 无活跃对象时兜底 AI', () => {
  const s = setup()
  assert.equal(s.streamingRoleLabel, 'AI')
})

test('applyQualityFromTurn 把后端 DTO 写入 pipeline.quality', () => {
  const s = setup()
  s.pipeline.state = 'done'
  s.applyQualityFromTurn({
    turn_id: 't1',
    attempt_id: 'a1',
    status: 'AwaitingAcceptance',
    passed: false,
    warning_count: 2,
    error_count: 1,
    warnings: ['n-gram 重复', '字数过短'],
  })
  assert.equal(s.pipeline.quality.passed, false)
  assert.equal(s.pipeline.quality.warningCount, 2)
  assert.equal(s.pipeline.quality.errorCount, 1)
  assert.deepEqual(s.pipeline.quality.warnings, ['n-gram 重复', '字数过短'])
  assert.equal(s.pipeline.quality.status, 'error')
  assert.equal(s.pipeline.quality.source, 'turn')
  assert.equal(s.pipeline.stateLabel, '已产出 · 质量警告 2')
})

test('applyQualityFromTurn(null) 清空 quality', () => {
  const s = setup()
  s.applyQualityFromTurn({
    passed: true,
    warning_count: 0,
    warnings: [],
  })
  assert.equal(s.pipeline.quality.passed, true)
  s.applyQualityFromTurn(null)
  assert.equal(s.pipeline.quality, null)
})

test('canChooseGreeting legacy + 无对话 + 多开场白时为 true', () => {
  setActivePinia(createPinia())
  const writing = useWritingStore()
  const campaign = useCampaignStore()
  campaign.activeChar = { id: 'c1' }
  // buildGreetingOptionsFromDetail 直接读 detail.first_mes / detail.alternate_greetings
  campaign.activeCharDetail = {
    first_mes: '开场',
    alternate_greetings: ['开场A', '开场B', '开场C'],
  }
  assert.equal(writing.writingMode, 'legacy')
  assert.ok(writing.greetingOptions.length >= 2, '应有多个开场白选项')
  assert.equal(writing.canChooseGreeting, true)
})

test('canChooseGreeting 有 conversation 时为 false', () => {
  setActivePinia(createPinia())
  const writing = useWritingStore()
  const campaign = useCampaignStore()
  campaign.activeChar = { id: 'c1' }
  campaign.activeCharDetail = { first_mes: 'x', alternate_greetings: ['a', 'b'] }
  campaign.currentConversationId = 'conv_1'
  assert.equal(writing.canChooseGreeting, false)
})

test('canChooseGreeting campaign 模式时为 false', () => {
  setActivePinia(createPinia())
  const writing = useWritingStore()
  const campaign = useCampaignStore()
  campaign.activeChar = { id: 'c1' }
  campaign.activeCharDetail = { first_mes: 'x', alternate_greetings: ['a', 'b'] }
  campaign.activeCampaign = { id: 'camp_1' }
  assert.equal(writing.writingMode, 'campaign')
  assert.equal(writing.canChooseGreeting, false)
})

test('selectedGreeting 按 index 取 greetingOptions', () => {
  setActivePinia(createPinia())
  const writing = useWritingStore()
  const campaign = useCampaignStore()
  campaign.activeCharDetail = { first_mes: '默认', alternate_greetings: ['A', 'B'] }
  writing.selectedGreetingIndex = 1
  assert.ok(writing.selectedGreeting, 'index 1 应有值')
  writing.selectedGreetingIndex = 99
  assert.equal(writing.selectedGreeting, null, '越界 index 返回 null')
})
