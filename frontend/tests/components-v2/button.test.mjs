// Button.vue 挂载测试(vitest)。
// vitest 用 vite vue 插件编译 SFC,happy-dom 提供 DOM,无需手写 loader。
import { test, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import Button from '../../src/components-v2/ui/Button.vue'

test('Button primary variant 渲染 accent 类', () => {
  const w = mount(Button, { props: { variant: 'primary' }, slots: { default: '发送' } })
  const cls = w.attributes('class') || ''
  expect(cls).toContain('accent')
  expect(w.text()).toContain('发送')
})

test('Button disabled 时 button 元素也 disabled', () => {
  const w = mount(Button, { props: { disabled: true }, slots: { default: 'x' } })
  expect(w.attributes('disabled')).toBeDefined()
})

test('Button loading 时不渲染 slot,显示 spinner', () => {
  const w = mount(Button, { props: { loading: true }, slots: { default: '发送' } })
  expect(w.text()).not.toContain('发送')
  expect(w.text()).toContain('◌')
})

test('Button 点击触发 click 事件', async () => {
  const w = mount(Button, { slots: { default: '点我' } })
  await w.trigger('click')
  expect(w.emitted('click')).toHaveLength(1)
})

test('Button size 类正确', () => {
  const w = mount(Button, { props: { size: 'lg' }, slots: { default: 'x' } })
  const cls = w.attributes('class') || ''
  expect(cls).toContain('px-5')
})

test('Button danger variant 渲染 err 类', () => {
  const w = mount(Button, { props: { variant: 'danger' }, slots: { default: '删除' } })
  const cls = w.attributes('class') || ''
  expect(cls).toContain('err')
})

test('Button 默认 variant 含 surface-2', () => {
  const w = mount(Button, { slots: { default: '默认' } })
  const cls = w.attributes('class') || ''
  expect(cls).toContain('surface-2')
})
