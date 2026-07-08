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

test('opens the mocked Tauri app and visible panels', async ({ page }) => {
  await page.goto('/')
  await expect(page.locator('#app')).toBeVisible()
  await expect(page.locator('main')).toBeVisible()
  await page.screenshot({ path: `${artifactDir}/main.png`, fullPage: true })

  await page.locator('header button').last().click()
  await expect(page.locator('[role="dialog"]').last()).toBeVisible()
  await expect(page.locator('aside')).toBeVisible()
  await page.screenshot({ path: `${artifactDir}/debug-drawer.png`, fullPage: true })
  await page.keyboard.press('Escape')
  await expect(page.locator('[role="dialog"]')).toHaveCount(0)

  await page.locator('header button').first().click()
  await expect(page.locator('nav')).toBeVisible()
  await page.screenshot({ path: `${artifactDir}/sidebar.png`, fullPage: true })

  await page.locator('nav button').nth(8).click()
  await expect(page.locator('[role="dialog"]').last()).toBeVisible()
  await expect(page.locator('[role="dialog"]').last()).toContainText('UI Smoke Plugin')
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
