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

// F-22：`portal` 是真实的 opt-out —— 默认（false）面板留在组件内，
// `portal: true` 才挂到 body。用 headlessui v1.7 的 `Portal` 做开关是错的
// （它只有 `as` 一个 prop，永远 teleport），这条测试锁住这个区别。
test('Select portal=false（默认）时选项面板留在组件内部', async () => {
  const w = mount(Select, { props: baseProps })
  await w.find('button').trigger('click')
  expect(w.find('[role="listbox"]').exists()).toBe(true)
  expect(document.body.querySelector('[role="listbox"]')).toBe(null)
  w.unmount()
})

test('Select portal=true 时选项面板挂到 body 且仍可选中', async () => {
  const w = mount(Select, { props: { ...baseProps, portal: true } })
  await w.find('button').trigger('click')
  expect(w.find('[role="listbox"]').exists()).toBe(false)
  const panel = document.body.querySelector('[role="listbox"]')
  expect(panel).not.toBe(null)
  const options = Array.from(document.body.querySelectorAll('[role="option"]'))
  options[1].dispatchEvent(new MouseEvent('click', { bubbles: true }))
  await new Promise((r) => setTimeout(r, 50))
  expect(w.emitted('update:modelValue')).toEqual([['乙']])
  w.unmount()
})
