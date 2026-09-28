// ui/ 组件挂载测试(批量,验证关键组件渲染正确)。
// vitest 自动编译 SFC + happy-dom 提供 DOM。
// F-18：SegmentedControl / DiffView 已作为死基元删除（零生产引用），对应断言一并移除。
import { test, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import Badge from '../../src/components-v2/ui/Badge.vue'
import Input from '../../src/components-v2/ui/Input.vue'
import Toggle from '../../src/components-v2/ui/Toggle.vue'
import EmptyState from '../../src/components-v2/ui/EmptyState.vue'
import DataTable from '../../src/components-v2/ui/DataTable.vue'
import Tabs from '../../src/components-v2/ui/Tabs.vue'
import CodeBlock from '../../src/components-v2/ui/CodeBlock.vue'

test('Badge variant ok 渲染 ok 类', () => {
  const w = mount(Badge, { props: { variant: 'ok' }, slots: { default: '通过' } })
  expect(w.attributes('class') || '').toContain('ok')
  expect(w.text()).toContain('通过')
})

test('Badge variant accent 渲染 accent 类', () => {
  const w = mount(Badge, { props: { variant: 'accent' }, slots: { default: '主' } })
  expect(w.attributes('class') || '').toContain('accent')
})

test('Input v-model 双向绑定', async () => {
  const w = mount(Input, { props: { modelValue: '初始', placeholder: '输入' } })
  expect(w.find('input').element.value).toBe('初始')
  await w.find('input').setValue('新值')
  expect(w.emitted('update:modelValue')?.[0]).toEqual(['新值'])
})

test('Input invalid 状态渲染 err 边框并输出 aria-invalid（F-38）', () => {
  const w = mount(Input, { props: { invalid: true } })
  expect(w.attributes('class') || '').toContain('err')
  expect(w.find('input').attributes('aria-invalid')).toBe('true')
})

test('Input 正常态不输出 aria-invalid', () => {
  const w = mount(Input, { props: { invalid: false } })
  expect(w.find('input').attributes('aria-invalid')).toBeUndefined()
})

test('Toggle 点击触发 update:modelValue', async () => {
  const w = mount(Toggle, { props: { modelValue: false } })
  await w.trigger('click')
  expect(w.emitted('update:modelValue')?.[0]).toEqual([true])
})

test('Toggle 关闭态渲染 line 类', () => {
  const w = mount(Toggle, { props: { modelValue: false } })
  const cls = w.attributes('class') || ''
  expect(cls).toContain('line')
})

test('Toggle 有可访问名（F-23 label/ariaLabel）', () => {
  const w = mount(Toggle, { props: { modelValue: false, label: '后处理' } })
  expect(w.attributes('aria-label')).toBe('后处理')
})

test('EmptyState 渲染 title 和 description', () => {
  const w = mount(EmptyState, {
    props: { title: '无 Campaign', description: '请新建一个' },
  })
  expect(w.text()).toContain('无 Campaign')
  expect(w.text()).toContain('请新建一个')
})

test('DataTable 渲染表头和单元格', () => {
  const w = mount(DataTable, {
    props: {
      columns: [{ key: 'name', label: '名称' }, { key: 'value', label: '值' }],
      rows: [{ name: 'HP', value: 100 }, { name: 'MP', value: 50 }],
    },
  })
  expect(w.text()).toContain('名称')
  expect(w.text()).toContain('HP')
  expect(w.text()).toContain('100')
  expect(w.findAll('tbody tr')).toHaveLength(2)
})

test('DataTable 空数据渲染 emptyTitle', () => {
  const w = mount(DataTable, {
    props: { columns: [{ key: 'x', label: 'X' }], rows: [], emptyTitle: '无知识条目' },
  })
  expect(w.text()).toContain('无知识条目')
})

test('Tabs 渲染所有 tab 标签', () => {
  const w = mount(Tabs, {
    props: {
      tabs: [{ key: 'instances', label: '实例' }, { key: 'knowledge', label: '知识' }],
      modelValue: 'instances',
    },
  })
  expect(w.text()).toContain('实例')
  expect(w.text()).toContain('知识')
})

test('CodeBlock 渲染代码内容和语言标签', () => {
  const w = mount(CodeBlock, {
    props: { code: '{"a": 1}', language: 'json' },
  })
  expect(w.text()).toContain('json')
  expect(w.text()).toContain('{"a": 1}')
})

// F-20/F-33/F-34：rowKey 去重键 + 空态 colspan 不再恒 +1 + maxItems 折叠
test('DataTable rowKey 生效：同 label 行用业务键而非下标', () => {
  const w = mount(DataTable, {
    props: {
      columns: [{ key: 'name', label: '名称' }],
      rows: [{ id: 'a', name: '同名' }, { id: 'b', name: '同名' }],
      rowKey: 'id',
    },
  })
  expect(w.findAll('tbody tr')).toHaveLength(2)
})

test('DataTable 无 row-action 插槽时空态 colspan 等于列数', () => {
  const w = mount(DataTable, {
    props: { columns: [{ key: 'x', label: 'X' }, { key: 'y', label: 'Y' }], rows: [] },
  })
  expect(w.find('tbody td').attributes('colspan')).toBe('2')
})

test('DataTable maxItems 折叠多余行并可展开', async () => {
  const rows = Array.from({ length: 5 }, (_, i) => ({ id: i, name: `r${i}` }))
  const w = mount(DataTable, {
    props: { columns: [{ key: 'name', label: '名称' }], rows, rowKey: 'id', maxItems: 2 },
  })
  expect(w.findAll('tbody tr')).toHaveLength(2)
  expect(w.text()).toContain('显示更多（还有 3 行）')
  await w.find('button').trigger('click')
  expect(w.findAll('tbody tr')).toHaveLength(5)
})
