// Select.vue 交互回归测试（vitest + happy-dom）。
// 背景：Listbox 重写后真实浏览器出现「选择后不收起」，
// 修复为 Vue 原生 Transition + aria-expanded 驱动 open 态，此测试锁定行为。
import { test, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import Select from '../../src/components-v2/ui/Select.vue'

const baseProps = {
  modelValue: null,
  options: ['甲', '乙', '丙'],
  placeholder: '请选择',
}

test('Select 点击触发器展开选项面板', async () => {
  const w = mount(Select, { props: baseProps })
  expect(w.find('[role="listbox"]').exists()).toBe(false)
  await w.find('button').trigger('click')
  expect(w.find('[role="listbox"]').exists()).toBe(true)
  expect(w.find('button').attributes('aria-expanded')).toBe('true')
})

test('Select 选择选项后 emit 值并收起面板', async () => {
  const w = mount(Select, { props: baseProps })
  await w.find('button').trigger('click')
  await w.findAll('[role="option"]')[1].trigger('click')
  await new Promise((r) => setTimeout(r, 50))
  expect(w.emitted('update:modelValue')).toEqual([['乙']])
  expect(w.find('[role="listbox"]').exists()).toBe(false)
  expect(w.find('button').attributes('aria-expanded')).toBe('false')
})

test('Select disabled 时不展开', async () => {
  const w = mount(Select, { props: { ...baseProps, disabled: true } })
  await w.find('button').trigger('click', { force: true })
  expect(w.find('[role="listbox"]').exists()).toBe(false)
})

test('Select 对象选项：label 展示、value 透传', async () => {
  const w = mount(Select, {
    props: {
      modelValue: 'a',
      options: [
        { label: '选项 A', value: 'a' },
        { label: '选项 B', value: 'b' },
      ],
    },
  })
  expect(w.find('button').text()).toContain('选项 A')
  await w.find('button').trigger('click')
  await w.findAll('[role="option"]')[1].trigger('click')
  expect(w.emitted('update:modelValue')).toEqual([['b']])
})
