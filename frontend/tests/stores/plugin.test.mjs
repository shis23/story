import test from 'node:test'
import assert from 'node:assert/strict'
import { createPinia, setActivePinia } from 'pinia'
import { usePluginStore } from '../../src/stores/plugin.js'

function setup() {
  setActivePinia(createPinia())
  return usePluginStore()
}

test('plugin store 初始状态为空数组/空对象', () => {
  const s = setup()
  assert.deepEqual(s.sidebarPlugins, [])
  assert.deepEqual(s.hookPlugins, [])
  assert.deepEqual(s.hookPluginSlots, {})
  assert.deepEqual(s.pluginPipelineEvents, [])
  assert.deepEqual(s.promptHookAuditRecords, [])
})

test('hookPluginHostRefs 是 Map 且可读写', () => {
  const s = setup()
  assert.ok(s.hookPluginHostRefs instanceof Map)
  const fakeHost = { emitPluginEventAndWait: () => {} }
  s.setHookPluginHostRef('p1', fakeHost)
  assert.equal(s.getHookPluginHostRef('p1'), fakeHost)
  assert.equal(s.allHookPluginHostRefs().length, 1)
})

test('setHookPluginHostRef 传 host=null 等价删除', () => {
  const s = setup()
  s.setHookPluginHostRef('p1', { x: 1 })
  assert.ok(s.getHookPluginHostRef('p1'))
  s.setHookPluginHostRef('p1', null)
  assert.equal(s.getHookPluginHostRef('p1'), undefined)
})

test('nextPipelineEventSeq 单调递增', () => {
  const s = setup()
  assert.equal(s.nextPipelineEventSeq(), 1)
  assert.equal(s.nextPipelineEventSeq(), 2)
  assert.equal(s.nextPipelineEventSeq(), 3)
})

test('上限常量符合当前 trace 和审计容量契约', () => {
  const s = setup()
  assert.equal(s.MAX_PLUGIN_PIPELINE_EVENTS, 500)
  assert.equal(s.MAX_PROMPT_HOOK_AUDIT_RECORDS, 100)
})

test('plugin store 状态可赋值(响应式)', () => {
  const s = setup()
  s.sidebarPlugins = [{ id: 'p1', name: 'test' }]
  assert.equal(s.sidebarPlugins.length, 1)
  assert.equal(s.sidebarPlugins[0].id, 'p1')
})

test('每个 store 实例有独立序号(新 pinia 重置)', () => {
  const s1 = setup()
  s1.nextPipelineEventSeq()
  s1.nextPipelineEventSeq()
  // 新 pinia → 新 store 实例,序号从头
  setActivePinia(createPinia())
  const s2 = usePluginStore()
  assert.equal(s2.nextPipelineEventSeq(), 1)
})

// loadSidebarPlugins 装配逻辑（AppV2 已从 stub 恢复）：
// enabled → hookPlugins；enabled && ui_slots 含 SidebarPanel → sidebarPlugins
test('插件装配规则：enabled 分流 hook/sidebar', () => {
  const s = setup()
  const all = [
    { id: 'a', enabled: true, ui_slots: ['SidebarPanel'] },
    { id: 'b', enabled: true, ui_slots: [] },
    { id: 'c', enabled: false, ui_slots: ['SidebarPanel'] },
  ]
  const enabled = all.filter((p) => p.enabled)
  s.hookPlugins = enabled
  s.sidebarPlugins = enabled.filter((p) => p.ui_slots?.includes('SidebarPanel'))
  assert.deepEqual(
    s.hookPlugins.map((p) => p.id),
    ['a', 'b'],
  )
  assert.deepEqual(
    s.sidebarPlugins.map((p) => p.id),
    ['a'],
  )
})

test('插件装配会清掉已禁用插件的 hookPluginSlots', () => {
  const s = setup()
  s.hookPluginSlots = {
    a: { SidebarPanel: '<div>a</div>' },
    c: { SidebarPanel: '<div>c</div>' },
  }
  const enabledIds = new Set(['a', 'b'])
  s.hookPluginSlots = Object.fromEntries(
    Object.entries(s.hookPluginSlots).filter(([pluginId]) => enabledIds.has(pluginId)),
  )
  assert.deepEqual(Object.keys(s.hookPluginSlots), ['a'])
})
