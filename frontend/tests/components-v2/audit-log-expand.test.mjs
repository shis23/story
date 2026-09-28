// PromptHookAuditLog 展开详情回归测试（F-41，P1）。
//
// 缺陷：toggleExpand(row) 内部写成 `recordKey(record)`，而形参是 row →
// 点击箭头 100% 抛 `ReferenceError: record is not defined`，生产调试抽屉里
// 唯一可读的 prompt hook 详情永远打不开（挂载调用方是 InspectorDrawer:16）。
// 同时覆盖 F-47：详情块改为"只渲染已展开项"（此前 v-for + v-show + index key）。
import { test, expect, beforeEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import PromptHookAuditLog from '../../src/components-v2/debug/PromptHookAuditLog.vue'
import { usePluginStore } from '../../src/stores/index.js'

function makeRecord(overrides = {}) {
  return {
    kind: 'prompt_hook',
    pluginId: 'demo.plugin',
    pluginName: '示例插件',
    event: 'CHAT_COMPLETION_PROMPT_READY',
    stage: 'llm_messages',
    status: 'ok',
    durationMs: 12,
    changedKeys: ['messages'],
    inputSummary: { messages: 2 },
    outputSummary: { messages: 2, changed: true },
    error: null,
    correlationId: 'hook:1:1',
    recordedAt: 1700000000000,
    ...overrides,
  }
}

function mountWith(records) {
  const pinia = createPinia()
  setActivePinia(pinia)
  const store = usePluginStore()
  store.promptHookAuditRecords = records
  return mount(PromptHookAuditLog, { global: { plugins: [pinia] } })
}

beforeEach(() => {
  setActivePinia(createPinia())
})

test('有记录时渲染表格行，未展开时不渲染详情', () => {
  const w = mountWith([makeRecord()])
  expect(w.text()).toContain('示例插件')
  expect(w.text()).not.toContain('输入摘要')
})

test('点击展开按钮不抛错且详情出现（F-41 回归）', async () => {
  const w = mountWith([makeRecord()])
  const expandButton = w.findAll('button').find((b) => b.attributes('title') === '展开')
  expect(expandButton).toBeTruthy()
  await expandButton.trigger('click')
  await flushPromises()
  expect(w.text()).toContain('输入摘要')
  expect(w.text()).toContain('输出摘要')
})

test('展开只渲染一条详情块，再次点击收起', async () => {
  const w = mountWith([
    makeRecord({ correlationId: 'a', recordedAt: 2 }),
    makeRecord({ correlationId: 'b', recordedAt: 1, pluginName: '另一个插件' }),
  ])
  const buttons = () => w.findAll('button')
  const first = buttons().find((b) => b.attributes('title') === '展开')
  await first.trigger('click')
  await flushPromises()
  expect(w.findAll('.bg-surface-2.rounded-lg.border.border-line.p-3')).toHaveLength(1)

  const collapse = buttons().find((b) => b.attributes('title') === '收起')
  expect(collapse).toBeTruthy()
  await collapse.trigger('click')
  await flushPromises()
  expect(w.text()).not.toContain('输入摘要')
})

test('无记录时显示空态', () => {
  const w = mountWith([])
  expect(w.text()).toContain('无审计记录')
})
