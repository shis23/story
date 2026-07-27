import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import StreamingBody from '../../src/design/writing/StreamingBody.vue'
import ProcessTimeline from '../../src/design/writing/ProcessTimeline.vue'
import ComposerBar from '../../src/design/writing/ComposerBar.vue'

const continuationPipeline = {
  state: 'done',
  director: { status: 'idle', detail: '', output: '' },
  subagents: [],
  editor: {
    role: 'writer',
    status: 'done',
    detail: '正文完成',
    output: '这是应该完整展示的正文，而不是最后三个字。',
  },
  quality: { passed: true, warningCount: 0, warnings: [] },
  summary: { status: 'done', detail: '摘要 128 字', output: '' },
  postprocess: {
    status: 'done',
    detail: '知识 2 · 变量 1 · 任务 0',
    knowledge: 2,
    variable: 1,
    task: 0,
  },
}

describe('writing pipeline presentation', () => {
  it('shows the real continuation writer and hides idle director/editor labels', async () => {
    const wrapper = mount(StreamingBody, {
      props: {
        pipeline: {
          ...continuationPipeline,
          quality: null,
          summary: { status: 'idle', detail: '', charCount: 0 },
          postprocess: { status: 'idle', detail: '', knowledge: 0, variable: 0, task: 0 },
        },
        generationMode: 'continuation',
      },
    })

    await wrapper.get('button').trigger('click')

    expect(wrapper.text()).toContain('执笔者')
    expect(wrapper.text()).not.toContain('导演')
    expect(wrapper.text()).not.toContain('编剧')
    expect(wrapper.text()).toContain('过程 1/1')
  })

  it('distinguishes model calls from deterministic checks and opens the full draft', async () => {
    const wrapper = mount(ProcessTimeline, {
      props: { pipeline: continuationPipeline, generationMode: 'continuation' },
    })

    await wrapper.get('button').trigger('click')
    expect(wrapper.text()).toContain('正文续写')
    expect(wrapper.text()).toContain('剧情摘要')
    expect(wrapper.text()).toContain('状态记账')
    expect(wrapper.text()).toContain('规则检查')
    expect(wrapper.text()).toContain('3 个模型阶段')

    const contentStep = wrapper.findAll('button').find((button) => button.text().includes('正文续写'))
    expect(contentStep).toBeTruthy()
    await contentStep.trigger('click')
    expect(wrapper.text()).toContain('这是应该完整展示的正文，而不是最后三个字。')
  })

  it('uses one focus surface without nested textarea outlines', () => {
    const wrapper = mount(ComposerBar, {
      props: { generationMode: 'continuation', showGenerationModes: true },
    })

    const shell = wrapper.get('[data-testid="composer-input-shell"]')
    const textarea = wrapper.get('textarea')
    expect(wrapper.text()).not.toContain('重点')
    expect(shell.classes()).toContain('rounded-xl')
    expect(shell.classes()).not.toContain('border')
    expect(shell.classes()).not.toContain('border-line')
    expect(textarea.classes()).toContain('composer-textarea')
    expect(textarea.classes()).toContain('border-0')
    expect(textarea.attributes('aria-label')).toBe('写作意图')
  })
})
