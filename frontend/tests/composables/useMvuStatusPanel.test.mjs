import test from 'node:test'
import assert from 'node:assert/strict'
import { nextTick } from 'vue'
import { createPinia, setActivePinia } from 'pinia'
import { useMvuStatusPanel } from '../../src/composables/useMvuStatusPanel.js'
import { useCampaignStore } from '../../src/stores/campaign.js'
import { useWritingStore } from '../../src/stores/writing.js'

const card = {
  id: 'card-1',
  source_character_id: 'source-1',
  character_definitions: [{ id: 'def-a', name: 'A' }],
}

const translationDetail = {
  source_character_id: 'source-1',
  translation: {
    ui_bindings: [{ element: 'hp-bar', variable_key: 'hp', display: { kind: 'bar', max: 100 } }],
    fallback_fragments: [],
    interactions: [
      {
        element_label: '攻击按钮',
        actions: [
          { kind: 'modify_variable', key: 'hp', value_expr: '-10' },
          { kind: 'trigger_next_turn', hint: '主角发起攻击' },
        ],
      },
    ],
  },
}

function makeApis(overrides = {}) {
  const calls = { getCard: 0, getMvu: 0, listInstances: 0, getCampaignVariables: 0 }
  return {
    calls,
    apis: {
      getCardApi: async () => {
        calls.getCard += 1
        return card
      },
      metaGetMvuTranslationApi: async () => {
        calls.getMvu += 1
        return translationDetail
      },
      listInstancesApi: async () => {
        calls.listInstances += 1
        return [
          { id: 'inst-a', name: '江离', definition_id: 'def-a', variables: [{ key: 'hp', value: 42 }] },
        ]
      },
      getCampaignVariablesApi: async () => {
        calls.getCampaignVariables += 1
        return []
      },
      ...overrides,
    },
  }
}

async function flushAsync() {
  await nextTick()
  await new Promise((resolve) => setTimeout(resolve, 0))
}

test('refresh builds sections from card -> translation -> instances chain', async () => {
  setActivePinia(createPinia())
  const campaign = useCampaignStore()
  campaign.activeCampaign = { id: 'camp-1', card_id: 'card-1' }
  const { apis } = makeApis()

  const { mvuStatusSections, refreshMvuStatusPanel } = useMvuStatusPanel(apis)
  await refreshMvuStatusPanel()

  assert.equal(mvuStatusSections.value.length, 1)
  assert.equal(mvuStatusSections.value[0].instanceId, 'inst-a')
  assert.equal(mvuStatusSections.value[0].instanceName, '江离')
  assert.deepEqual(mvuStatusSections.value[0].mvuState.uiBindings, translationDetail.translation.ui_bindings)
})

test('refresh clears sections without active campaign or translation', async () => {
  setActivePinia(createPinia())
  const campaign = useCampaignStore()
  campaign.activeCampaign = null
  const { apis } = makeApis()

  const { mvuStatusSections, refreshMvuStatusPanel } = useMvuStatusPanel(apis)
  await refreshMvuStatusPanel()
  assert.deepEqual(mvuStatusSections.value, [])

  campaign.activeCampaign = { id: 'camp-1', card_id: 'card-1' }
  const noTranslation = makeApis({ metaGetMvuTranslationApi: async () => null })
  const second = useMvuStatusPanel(noTranslation.apis)
  await second.refreshMvuStatusPanel()
  assert.deepEqual(second.mvuStatusSections.value, [])
})

test('refresh survives api failure by clearing sections', async () => {
  setActivePinia(createPinia())
  const campaign = useCampaignStore()
  campaign.activeCampaign = { id: 'camp-1', card_id: 'card-1' }
  const failing = makeApis({
    getCardApi: async () => {
      throw new Error('boom')
    },
  })

  const { mvuStatusSections, refreshMvuStatusPanel } = useMvuStatusPanel(failing.apis)
  await refreshMvuStatusPanel()
  assert.deepEqual(mvuStatusSections.value, [])
})

test('refresh exposes card-level interaction mappings', async () => {
  setActivePinia(createPinia())
  const campaign = useCampaignStore()
  campaign.activeCampaign = { id: 'camp-1', card_id: 'card-1' }
  const { apis } = makeApis()

  const { mvuInteractionMappings, refreshMvuStatusPanel } = useMvuStatusPanel(apis)
  await refreshMvuStatusPanel()

  assert.equal(mvuInteractionMappings.value.length, 1)
  assert.equal(mvuInteractionMappings.value[0].element_label, '攻击按钮')
})

test('dispatch writes to the single bound instance scope then triggers next turn', async () => {
  setActivePinia(createPinia())
  const campaign = useCampaignStore()
  campaign.activeCampaign = { id: 'camp-1', card_id: 'card-1' }
  const { apis } = makeApis()

  const persisted = []
  const hints = []
  const { mvuInteractionMappings, dispatchMvuInteraction, refreshMvuStatusPanel } =
    useMvuStatusPanel({
      ...apis,
      persistWriteApi: async (args) => {
        persisted.push(args)
        return { ok: true, scope: 'instance', key: args.key }
      },
      startWriting: async (hint) => {
        hints.push(hint)
      },
    })
  await refreshMvuStatusPanel()

  const outcome = await dispatchMvuInteraction(mvuInteractionMappings.value[0])

  assert.equal(persisted.length, 1)
  assert.equal(persisted[0].campaignId, 'camp-1')
  // 恰好一个卡绑定实例 → instance 作用域前缀
  assert.equal(persisted[0].key, 'instance:inst-a:hp')
  // hp 当前 42，"-10" 按增量解释
  assert.equal(persisted[0].value, 32)
  assert.deepEqual(hints, ['主角发起攻击'])
  assert.equal(outcome.plan.writes.length, 1)
  assert.equal(outcome.results[0].ok, true)
})

test('dispatch falls back to the single campaign instance when no section renders', async () => {
  setActivePinia(createPinia())
  const campaign = useCampaignStore()
  campaign.activeCampaign = { id: 'camp-1', card_id: 'card-1' }
  // 有交互但无可渲染节（实例未绑定 definition）；全 campaign 恰一个实例
  campaign.instanceNameMap = { 'inst-a': '江离' }
  const { apis } = makeApis({
    listInstancesApi: async () => [
      { id: 'inst-a', name: '江离', definition_id: null, variables: [] },
    ],
  })

  const persisted = []
  const panel = useMvuStatusPanel({
    ...apis,
    persistWriteApi: async (args) => {
      persisted.push(args)
      return { ok: true, scope: 'instance', key: args.key }
    },
  })
  await panel.refreshMvuStatusPanel()
  assert.equal(panel.mvuStatusSections.value.length, 0)

  await panel.dispatchMvuInteraction({
    element_label: '修炼',
    actions: [{ kind: 'modify_variable', key: 'exp', value_expr: '+10' }],
  })
  assert.equal(persisted.length, 1)
  // 回退判据与 onShellVarWrite 一致：全 campaign 单实例 → instance 作用域
  assert.equal(persisted[0].key, 'instance:inst-a:exp')
})

test('dispatch skips writing-in-progress trigger and returns null without campaign', async () => {
  setActivePinia(createPinia())
  const campaign = useCampaignStore()
  const writing = useWritingStore()
  campaign.activeCampaign = { id: 'camp-1', card_id: 'card-1' }
  const { apis } = makeApis()

  const hints = []
  const panel = useMvuStatusPanel({
    ...apis,
    persistWriteApi: async () => ({ ok: true, scope: 'instance', key: 'hp' }),
    startWriting: async (hint) => {
      hints.push(hint)
    },
  })
  await panel.refreshMvuStatusPanel()

  writing.isWriting = true
  await panel.dispatchMvuInteraction(panel.mvuInteractionMappings.value[0])
  assert.deepEqual(hints, [], '写作进行中不应触发 trigger_next_turn')
  writing.isWriting = false

  campaign.activeCampaign = null
  const result = await panel.dispatchMvuInteraction(panel.mvuInteractionMappings.value[0])
  assert.equal(result, null)
})

test('campaign switch and postprocess running->done both trigger refresh', async () => {
  setActivePinia(createPinia())
  const campaign = useCampaignStore()
  const writing = useWritingStore()
  campaign.activeCampaign = null
  const { apis, calls } = makeApis()

  useMvuStatusPanel(apis)
  await flushAsync()
  const baseline = calls.getCard

  campaign.activeCampaign = { id: 'camp-1', card_id: 'card-1' }
  await flushAsync()
  assert.ok(calls.getCard > baseline, 'campaign 切换应触发刷新')

  const afterSwitch = calls.getCard
  writing.pipeline.postprocess.status = 'running'
  await flushAsync()
  writing.pipeline.postprocess.status = 'done'
  await flushAsync()
  assert.ok(calls.getCard > afterSwitch, 'postprocess running→done 应触发刷新（变量已更新）')
})
