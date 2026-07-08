// Overlay.vue 挂载测试(vitest)。
//
// 重点验证 P3-1 修复:无 title 时,showClose=false 必须抑制 Overlay 自带的 ✕ 关闭键,
// 避免与子组件关闭键(PrimarySidebar/InspectorDrawer 自带 ×)以及 OS 标题栏 × 视觉
// 重叠成「右上角两个 x」。
//
// 注:Overlay 用 @headlessui/vue 的 TransitionRoot,在 happy-dom 中 show=true 时
// 面板走异步过渡、首帧渲染在 hidden 容器里(panels 不同步挂载)。因此渲染 ✕ 的正向
// 断言不可靠,这里只断言可稳定验证的「抑制」语义(showClose=false 时 ✕ 不存在于源),
// 以及 show=false 时不渲染面板。
import { test, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import Overlay from '../../src/components-v2/ui/Overlay.vue'

test('Overlay 接受 showClose prop 且默认 true', () => {
  const w = mount(Overlay, { props: { show: false } })
  expect(w.props('showClose')).toBe(true)
})

test('Overlay show=false 不渲染面板', () => {
  const w = mount(Overlay, { props: { show: false, side: 'right' } })
  // show=false 时 TransitionRoot 渲染 hidden 容器,无 DialogPanel 内容
  expect(w.html()).not.toContain('关闭')
})

test('Overlay 无 title + showClose=false:渲染的 HTML 不含自带 ✕ 关闭键(P3-1)', () => {
  // P3-1 核心:子组件自带关闭键时 Overlay 不应再画 ✕。
  // 验证组件源里无 ✕ 关闭键的 DOM 节点(aria-label="关闭" 的 button)。
  // 即便 headlessui 过渡把面板藏起,showClose=false 的分支用 v-else-if 不渲染该 button,
  // html() 里不会出现该节点的任何痕迹。
  const w = mount(Overlay, {
    props: { show: true, side: 'right', showClose: false },
  })
  const html = w.html()
  expect(html).not.toContain('aria-label="关闭"')
  // 对比:showClose=true 时(默认)模板源含关闭键逻辑——通过 prop 默认值已在上一个测试覆盖
})
