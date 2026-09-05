import { expect, test } from '@playwright/test'

const artifactDir = process.env.UI_SMOKE_ARTIFACT_DIR || 'artifacts/ui-smoke/local'

const mockPlugins = [
  {
    id: 'ui-smoke-plugin',
    name: 'UI Smoke Plugin',
    version: '0.0.1',
    description: 'Fixture plugin supplied by the Playwright Tauri IPC mock.',
    author: 'StoryForge',
    enabled: true,
    permissions: ['chat:read'],
    ui_slots: ['SidebarPanel'],
    manifest: {
      name: 'UI Smoke Plugin',
      entry_html: '<div data-ui-smoke-plugin-slot>Smoke plugin slot</div>',
    },
  },
]

const mockConnections = [
  {
    id: 'mock-conn',
    name: 'Mock Local LLM',
    base_url: 'http://127.0.0.1:11434/v1',
    protocol: 'OpenAI',
    model: 'mock-model',
    tool_mode: 'native',
    active: true,
  },
]

const mockModules = [
  {
    id: 'mock-style',
    category: 'Style',
    name: 'Smoke Style',
    content: 'Short smoke-test style module.',
    enabled: true,
  },
  {
    id: 'mock-output',
    category: 'Output',
    name: 'Smoke Output',
    content: 'Short smoke-test output module.',
    enabled: true,
  },
]

const mockProfile = {
  id: 'builtin-default-agent-v1',
  name: 'Smoke Default',
  description: 'Browser smoke profile',
  source: 'BuiltIn',
  config_version: 1,
  max_concurrent_subagents: 1,
  enable_postprocess: true,
  enable_summarizer: true,
  selections: {
    Editor: {
      Style: ['mock-style'],
      Output: ['mock-output'],
    },
    'Subagent:*': {
      Output: ['mock-output'],
    },
  },
  agent_configs: {},
}

const handlers = {
  get_version: () => 'ui-smoke',
  get_active_connection: () => mockConnections[0],
  list_connections: () => mockConnections,
  list_connection_templates: () => [],
  list_models: () => [],
  list_conversations: () => [],
  get_conversation: () => null,
  get_active_campaign: () => null,
  list_plugins: () => mockPlugins,
  install_plugin: () => null,
  uninstall_plugin: () => null,
  set_plugin_enabled: () => null,
  list_modules: () => mockModules,
  update_module: () => null,
  get_active_profile: () => mockProfile,
  save_profile: () => null,
  list_agent_profile_configs: () => [
    {
      id: mockProfile.id,
      name: mockProfile.name,
      description: mockProfile.description,
      source: mockProfile.source,
      is_active: true,
    },
  ],
  get_agent_profile_config: () => mockProfile,
  get_active_agent_profile_config: () => mockProfile,
  save_agent_profile_config: () => null,
  export_agent_profile_config: () => JSON.stringify(mockProfile),
  import_agent_profile_config: () => mockProfile,
  delete_agent_profile_config: () => null,
  set_active_agent_profile_config: () => null,
  list_presets: () => [],
  get_active_preset: () => null,
  list_global_regex_scripts: () => [],
  list_cards: () => [],
  get_card: () => null,
  list_campaigns: () => [],
  list_instances: () => [],
  log_query: () => [],
  log_clear: () => null,
  log_export_bundle: () => ({ exported_at: new Date().toISOString(), counts: {} }),
  log_append_frontend: () => null,
  mvu_load_ack: () => null,
  mvu_unload_ack: () => null,
  mvu_execute_result: () => null,
}

async function installTauriInvokeMock(page) {
  await page.addInitScript(({ handlerNames }) => {
    const calls = []
    const mockValue = (command) => {
      switch (command) {
        case 'get_version':
          return 'ui-smoke'
        case 'get_active_connection':
          return {
            id: 'mock-conn',
            name: 'Mock Local LLM',
            base_url: 'http://127.0.0.1:11434/v1',
            protocol: 'OpenAI',
            model: 'mock-model',
            tool_mode: 'native',
            active: true,
          }
        case 'list_connections':
          return [{
            id: 'mock-conn',
            name: 'Mock Local LLM',
            base_url: 'http://127.0.0.1:11434/v1',
            protocol: 'OpenAI',
            model: 'mock-model',
            tool_mode: 'native',
            active: true,
          }]
        case 'list_plugins':
          return [{
            id: 'ui-smoke-plugin',
            name: 'UI Smoke Plugin',
            version: '0.0.1',
            description: 'Fixture plugin supplied by the Playwright Tauri IPC mock.',
            author: 'StoryForge',
            enabled: true,
            permissions: ['chat:read'],
            ui_slots: ['SidebarPanel'],
            manifest: { name: 'UI Smoke Plugin' },
          }]
        case 'list_modules':
          return [
            { id: 'mock-style', category: 'Style', name: 'Smoke Style', content: 'Short smoke-test style module.', enabled: true },
            { id: 'mock-output', category: 'Output', name: 'Smoke Output', content: 'Short smoke-test output module.', enabled: true },
          ]
        case 'get_active_profile':
        case 'get_agent_profile_config':
        case 'get_active_agent_profile_config':
          return {
            id: 'builtin-default-agent-v1',
            name: 'Smoke Default',
            description: 'Browser smoke profile',
            source: 'BuiltIn',
            config_version: 1,
            max_concurrent_subagents: 1,
            enable_postprocess: true,
            enable_summarizer: true,
            selections: { Editor: { Style: ['mock-style'], Output: ['mock-output'] }, 'Subagent:*': { Output: ['mock-output'] } },
            agent_configs: {},
          }
        case 'list_agent_profile_configs':
          return [{ id: 'builtin-default-agent-v1', name: 'Smoke Default', description: 'Browser smoke profile', source: 'BuiltIn', is_active: true }]
        case 'export_agent_profile_config':
          return '{}'
        case 'log_export_bundle':
          return { exported_at: new Date().toISOString(), counts: {} }
        case 'list_connections_templates':
        case 'list_connection_templates':
        case 'list_models':
        case 'list_conversations':
        case 'list_presets':
        case 'list_global_regex_scripts':
        case 'list_cards':
        case 'list_campaigns':
        case 'list_instances':
        case 'log_query':
          return []
        case 'get_conversation':
        case 'get_active_campaign':
        case 'get_active_preset':
        case 'get_card':
          return null
        default:
          if (!handlerNames.includes(command)) {
            console.warn(`[ui-smoke] unhandled Tauri IPC command: ${command}`)
          }
          return null
      }
    }

    window.__UI_SMOKE_TAURI_CALLS__ = calls
    window.__TAURI_INTERNALS__ = {
      invoke(command, args) {
        calls.push({ command, args })
        return Promise.resolve(mockValue(command))
      },
      transformCallback(callback) {
        const id = calls.length + 1
        window[`_${id}`] = callback
        return id
      },
    }
  }, { handlerNames: Object.keys(handlers) })
}

test.beforeEach(async ({ page }) => {
  await installTauriInvokeMock(page)
})

// AppV2 走查：桌面宽度下侧栏 docked 常显、调试抽屉是 docked 面板（非 dialog）、
// 插件面板 inline 挂载。定位一律用 role/name，避免对 DOM 结构顺序的脆弱假设。
test('opens the mocked Tauri app and visible panels', async ({ page }) => {
  await page.goto('/')
  await expect(page.locator('#app')).toBeVisible()
  await expect(page.locator('main')).toBeVisible()
  await page.screenshot({ path: `${artifactDir}/main.png`, fullPage: true })

  // 侧栏导航（desktop docked 常显）
  await expect(page.getByRole('navigation')).toBeVisible()
  await expect(page.getByRole('button', { name: 'Campaign 管理' })).toBeVisible()
  await page.screenshot({ path: `${artifactDir}/sidebar.png`, fullPage: true })

  // 调试抽屉：过程与调试 → 流水线 tab 出现
  await page.getByRole('button', { name: '过程与调试' }).click()
  await expect(page.getByRole('tab', { name: '流水线' })).toBeVisible()
  await page.screenshot({ path: `${artifactDir}/debug-drawer.png`, fullPage: true })
  // 关掉抽屉（overlay 模式的背景幕会拦截后续点击）
  await page.getByRole('button', { name: '关闭' }).first().click()
  await expect(page.getByRole('tab', { name: '流水线' })).toBeHidden()

  // 插件面板：mock 的插件名可见
  await page.getByRole('button', { name: '插件', exact: true }).click()
  await expect(page.getByText('UI Smoke Plugin').first()).toBeVisible()
  await page.screenshot({ path: `${artifactDir}/plugin-panel.png`, fullPage: true })

  const calls = await page.evaluate(() => window.__UI_SMOKE_TAURI_CALLS__.map((call) => call.command))
  expect(calls).toEqual(expect.arrayContaining([
    'get_version',
    'get_active_connection',
    'list_conversations',
    'get_active_campaign',
    'list_plugins',
  ]))
})

test('campaign title and summaries remain readable at desktop and narrow widths', async ({ page }) => {
  const title = 'ArchitectureReviewCampaignWithAnUnbrokenLongName'
  const summary = 'A complete summary remains readable across the available width. '.repeat(8)
  await page.addInitScript(({ title, summary }) => {
    const invoke = window.__TAURI_INTERNALS__.invoke
    const campaign = {
      id: 'layout-campaign', card_id: 'layout-card', name: title,
      instance_count: 0, conversation_id: 'layout-conversation', story_clock: 'Day 1',
    }
    window.__TAURI_INTERNALS__.invoke = (command, args) => {
      const values = {
        list_cards: [{ id: 'layout-card', name: 'Layout card', source_character_id: 'layout-source', character_count: 1 }],
        list_campaigns: [campaign],
        get_campaign: campaign,
        list_round_summaries: [{ id: 'layout-summary', turn: 1, content: summary, created_at: '2026-09-05T00:00:00Z' }],
        list_tasks: [],
        list_character_knowledge: [],
      }
      return command in values ? Promise.resolve(values[command]) : invoke(command, args)
    }
  }, { title, summary })
  await page.goto('/')
  await page.getByRole('button', { name: 'Campaign 管理' }).click()
  const campaignButton = page.getByRole('button', { name: new RegExp(title) })
  if (await campaignButton.isVisible()) await campaignButton.click()
  await expect(page.getByRole('heading', { name: title, exact: true, level: 2 })).toBeVisible()
  await page.getByRole('button', { name: '总结', exact: true }).click()
  await expect(page.getByText(summary, { exact: true }).last()).toBeVisible()
  await page.screenshot({ path: `${artifactDir}/campaign-desktop.png`, fullPage: true })
  await page.setViewportSize({ width: 390, height: 844 })
  const heading = page.getByRole('heading', { name: title, exact: true, level: 2 })
  const actions = page.getByRole('button', { name: '设为当前活动', exact: true })
  const h = await heading.boundingBox()
  const a = await actions.boundingBox()
  expect(h.y + h.height).toBeLessThanOrEqual(a.y)
  const mobile = page.getByTestId('summary-mobile')
  await expect(mobile).toBeVisible()
  expect((await mobile.locator('p').boundingBox()).width).toBeGreaterThan(250)
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
  await page.screenshot({ path: `${artifactDir}/campaign-narrow.png`, fullPage: true })
})
