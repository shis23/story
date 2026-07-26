/**
 * CardStudio 挂载冒烟（happy-dom + mock Tauri IPC）。
 * 覆盖：空列表态、项目列表渲染、打开项目后阶段条渲染、从零创建调用链。
 * 858 行组件此前零测试——本冒烟保证挂载不炸 + 主要数据流接线正确。
 */
import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import CardStudio from '../../src/components-v2/campaign/CardStudio.vue'

function makeProject(overrides = {}) {
  return {
    id: 'proj-1',
    name: '雾都侦探',
    brief: '雾都悬疑侦探卡',
    mode: 'from_scratch',
    current_stage: 'brief',
    stage_pack_id: 'mingyue_qiuqing_v1',
    stage_status: {
      brief: 'done',
      basic: 'pending',
      personality: 'pending',
      worldview: 'pending',
      opening: 'pending',
      review: 'pending',
      compile_import: 'pending',
    },
    artifacts: {
      name: '雾都侦探',
      description: '',
      personality: '',
      scenario: '',
      first_mes: '',
      tags: [],
      creator: '',
      worldview_entries: [],
      notes: '',
      personality_mode: null,
      personality_prompts: [],
      world_type: null,
      opening_outline: null,
      style_notes: null,
    },
    allow_ai_freewrite: false,
    last_error: null,
    updated_at: '2026-07-27T00:00:00Z',
    ...overrides,
  }
}

function installTauriMock({ projects = [] } = {}) {
  const calls = []
  const state = { projects: [...projects] }
  const mockValue = (command, args) => {
    switch (command) {
      case 'cardstudio_list_projects':
        return state.projects.map((p) => ({
          id: p.id,
          name: p.name,
          mode: p.mode,
          current_stage: p.current_stage,
          updated_at: p.updated_at,
        }))
      case 'cardstudio_get_project':
        return state.projects.find((p) => p.id === args?.id) || null
      case 'cardstudio_create_project': {
        const created = makeProject({
          id: 'proj-created',
          name: args?.name || '未命名',
          brief: args?.brief || '',
        })
        state.projects.push(created)
        return created
      }
      default:
        return null
    }
  }
  globalThis.__TAURI_INTERNALS__ = {
    invoke(command, args) {
      calls.push({ command, args })
      return Promise.resolve(mockValue(command, args))
    },
    transformCallback(callback) {
      const id = calls.length + 1
      globalThis[`_${id}`] = callback
      return id
    },
  }
  return calls
}

describe('CardStudio mount smoke', () => {
  let wrapper

  beforeEach(() => {
    localStorage.clear()
  })

  afterEach(async () => {
    if (wrapper) {
      wrapper.unmount()
      await flushPromises()
    }
    wrapper = null
    delete globalThis.__TAURI_INTERNALS__
  })

  it('mounts with an empty project list and shows the empty state', async () => {
    const calls = installTauriMock({ projects: [] })
    wrapper = mount(CardStudio)
    await flushPromises()

    expect(wrapper.text()).toContain('写卡工作室')
    expect(wrapper.text()).toContain('还没有写卡项目')
    expect(calls.map((c) => c.command)).toContain('cardstudio_list_projects')
  })

  it('renders project rows and opens a project with its stage strip', async () => {
    const calls = installTauriMock({ projects: [makeProject()] })
    wrapper = mount(CardStudio)
    await flushPromises()

    expect(wrapper.text()).toContain('雾都侦探')
    expect(wrapper.text()).toContain('从零')

    // 点开项目 → get_project → 阶段条渲染（意图/角色基础/…）
    const row = wrapper
      .findAll('button')
      .find((b) => b.text().includes('雾都侦探'))
    expect(row).toBeTruthy()
    await row.trigger('click')
    await flushPromises()

    expect(calls.some((c) => c.command === 'cardstudio_get_project' && c.args?.id === 'proj-1')).toBe(true)
    expect(wrapper.text()).toContain('意图 · done')
    expect(wrapper.text()).toContain('角色基础 · pending')
    expect(wrapper.text()).toContain('mingyue_qiuqing_v1')
  })

  it('creates a from-scratch project through the form', async () => {
    const calls = installTauriMock({ projects: [] })
    wrapper = mount(CardStudio)
    await flushPromises()

    const inputs = wrapper.findAll('input')
    await inputs[0].setValue('新项目甲')
    const textareas = wrapper.findAll('textarea')
    await textareas[0].setValue('测试意图 brief')

    const createBtn = wrapper
      .findAll('button')
      .find((b) => b.text().includes('从零创建'))
    expect(createBtn).toBeTruthy()
    await createBtn.trigger('click')
    await flushPromises()

    const createCall = calls.find((c) => c.command === 'cardstudio_create_project')
    expect(createCall?.args).toMatchObject({ name: '新项目甲', brief: '测试意图 brief' })
    // 创建成功后项目被打开（阶段条出现）且状态文案更新
    expect(wrapper.text()).toContain('已创建写卡项目')
    expect(wrapper.text()).toContain('意图 · done')
  })
})
