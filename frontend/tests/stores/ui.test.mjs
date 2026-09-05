import test from 'node:test'
import assert from 'node:assert/strict'
import { createPinia, setActivePinia } from 'pinia'
import { useUiStore } from '../../src/stores/ui.js'
import { useCampaignStore } from '../../src/stores/campaign.js'

function setup() {
  setActivePinia(createPinia())
  return useUiStore()
}

test('ui store 初始状态', () => {
  const s = setup()
  assert.equal(s.powerMode, false)
  assert.equal(s.appVersion, '...')
  assert.equal(s.importError, '')
  assert.equal(s.showCharList, false)
  assert.equal(s.showCampaignPanel, false)
  assert.equal(s.campaignPanelTab, 'instances')
  assert.equal(s.showMetaPanel, false)
  assert.equal(s.showPresetPanel, false)
  assert.equal(s.showPluginPanel, false)
  assert.equal(s.showDebugDrawer, false)
  assert.equal(s.showSidebar, false)
  assert.equal(s.sidebarCollapsed, false)
  assert.equal(s.showConnConfig, false)
  assert.equal(s.showHistory, true)
  assert.equal(s.activeCampaignOverview, true)
})

test('currentView 默认(showHistory=true + 无 campaign)为 history', () => {
  const s = setup()
  // 初始 showHistory=true, activeCampaignOverview=true, 但无 activeCampaign
  assert.equal(s.currentView, 'history')
})

test('currentView showHistory=false 时为 write', () => {
  const s = setup()
  s.showHistory = false
  assert.equal(s.currentView, 'write')
})

test('currentView showHistory=true + activeCampaign + overview=true 时为 overview', () => {
  setActivePinia(createPinia())
  const ui = useUiStore()
  const campaign = useCampaignStore()
  campaign.activeCampaign = { id: 'c1', name: '寒渊谜塔' }
  ui.showHistory = true
  ui.activeCampaignOverview = true
  assert.equal(ui.currentView, 'overview')
})

test('currentView showHistory=true + activeCampaign + overview=false 时为 history', () => {
  setActivePinia(createPinia())
  const ui = useUiStore()
  const campaign = useCampaignStore()
  campaign.activeCampaign = { id: 'c1', name: '寒渊谜塔' }
  ui.showHistory = true
  ui.activeCampaignOverview = false
  assert.equal(ui.currentView, 'history')
})

test('viewHistory 切到 history 视图', () => {
  const s = setup()
  s.viewHistory()
  assert.equal(s.showHistory, true)
  assert.equal(s.activeCampaignOverview, false)
})

test('viewOverview 切到 overview 视图', () => {
  const s = setup()
  s.viewOverview()
  assert.equal(s.showHistory, true)
  assert.equal(s.activeCampaignOverview, true)
})

test('viewWrite 切到 write 视图', () => {
  const s = setup()
  s.viewWrite()
  assert.equal(s.showHistory, false)
})

test('openCampaignPanel 可从写作页直达总结或变量', () => {
  const s = setup()

  s.openCampaignPanel('summaries')
  assert.equal(s.showCampaignPanel, true)
  assert.equal(s.campaignPanelTab, 'summaries')

  s.showCampaignPanel = false
  s.openCampaignPanel('variables')
  assert.equal(s.showCampaignPanel, true)
  assert.equal(s.campaignPanelTab, 'variables')
})

test('openCampaignPanel 对未知标签回退到实例', () => {
  const s = setup()
  s.openCampaignPanel('unknown')
  assert.equal(s.showCampaignPanel, true)
  assert.equal(s.campaignPanelTab, 'instances')
})

test('togglePower 开启后同时打开 debug drawer', () => {
  const s = setup()
  assert.equal(s.powerMode, false)
  assert.equal(s.showDebugDrawer, false)
  s.togglePower()
  assert.equal(s.powerMode, true)
  assert.equal(s.showDebugDrawer, true)
})

test('togglePower 关闭时不强制关 debug drawer(只切 powerMode)', () => {
  const s = setup()
  s.togglePower() // 开
  s.togglePower() // 关
  assert.equal(s.powerMode, false)
  // App.vue 原逻辑:关闭时不动 showDebugDrawer(保持原状)
})

test('pageTitle overview 视图用 campaign 名', () => {
  setActivePinia(createPinia())
  const ui = useUiStore()
  const campaign = useCampaignStore()
  campaign.activeCampaign = { id: 'c1', name: '寒渊谜塔' }
  ui.showHistory = true
  ui.activeCampaignOverview = true
  assert.equal(ui.currentView, 'overview')
  assert.equal(ui.pageTitle, '寒渊谜塔')
})

test('pageTitle history 视图为"会话历史"', () => {
  const s = setup()
  s.activeCampaignOverview = false
  assert.equal(s.currentView, 'history')
  assert.equal(s.pageTitle, '会话历史')
})

test('pageTitle write 视图无活跃对象时为"StoryForge"', () => {
  const s = setup()
  s.showHistory = false
  assert.equal(s.currentView, 'write')
  assert.equal(s.pageTitle, 'StoryForge')
})
