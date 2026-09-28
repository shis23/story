// ComposerBar 输入法组合态守卫（F-01）回归测试。
//
// 缺陷：`@keydown.enter.exact.prevent="submit"` 只校验修饰键、不校验输入法组合态，
// 中文拼音候选态按 Enter 确认候选词会直接提交半截文本并清空输入框。
// 守护：组合态（isComposing / compositionstart..compositionend / keyCode 229）
// 一律不得提交，也不得 preventDefault；裸 Enter 仍提交。
import { test, expect } from 'vitest'
import { nextTick } from 'vue'
import { mount } from '@vue/test-utils'
import ComposerBar from '../../src/design/writing/ComposerBar.vue'

function mountBar() {
  return mount(ComposerBar, {
    props: { writing: false, disabled: false },
  })
}

function pressEnter(textarea, patch = {}) {
  const event = new KeyboardEvent('keydown', {
    key: 'Enter',
    bubbles: true,
    cancelable: true,
  })
  // happy-dom 对 KeyboardEventInit 的支持不完整（keyCode 非标准字段、isComposing
  // 可能被忽略），这里显式定义，确保测的是"组件如何响应 IME 事件"而不是"DOM 实现
  // 是否支持该 init 字段"。
  if (patch.isComposing !== undefined) {
    Object.defineProperty(event, 'isComposing', { get: () => patch.isComposing })
  }
  if (patch.keyCode !== undefined) {
    Object.defineProperty(event, 'keyCode', { get: () => patch.keyCode })
  }
  for (const mod of ['shiftKey', 'ctrlKey', 'altKey', 'metaKey']) {
    if (patch[mod] !== undefined) {
      Object.defineProperty(event, mod, { get: () => patch[mod] })
    }
  }
  textarea.element.dispatchEvent(event)
  return event
}

test('组合态（isComposing=true）按 Enter 不提交', async () => {
  const w = mountBar()
  await w.find('textarea').setValue('下雨了')
  const event = pressEnter(w.find('textarea'), { isComposing: true })
  expect(w.emitted('start-writing')).toBeUndefined()
  expect(event.defaultPrevented).toBe(false)
  // 文本必须留在输入框里
  expect(w.find('textarea').element.value).toBe('下雨了')
})

test('compositionstart 之后、compositionend 之前的 Enter 不提交', async () => {
  const w = mountBar()
  const textarea = w.find('textarea')
  await textarea.setValue('候选词')
  await textarea.trigger('compositionstart')
  pressEnter(textarea)
  expect(w.emitted('start-writing')).toBeUndefined()
  expect(textarea.element.value).toBe('候选词')
})

test('compositionend 之后的裸 Enter 正常提交并清空', async () => {
  const w = mountBar()
  const textarea = w.find('textarea')
  await textarea.setValue('继续写')
  await textarea.trigger('compositionstart')
  await textarea.trigger('compositionend')
  const event = pressEnter(textarea)
  expect(w.emitted('start-writing')?.[0]).toEqual(['继续写'])
  expect(event.defaultPrevented).toBe(true)
  // submit() 里 `intent.value = ''` 经 v-model 回写 DOM 是异步的
  await nextTick()
  expect(textarea.element.value).toBe('')
})

test('旧式 IME 的 keyCode 229 伪键不提交（库/SDK 未设置 isComposing 时）', async () => {
  const w = mountBar()
  const textarea = w.find('textarea')
  await textarea.setValue('半截拼音')
  const event = pressEnter(textarea, { keyCode: 229 })
  expect(w.emitted('start-writing')).toBeUndefined()
  expect(event.defaultPrevented).toBe(false)
})

test('Shift+Enter 不提交（保持换行）', async () => {
  const w = mountBar()
  const textarea = w.find('textarea')
  await textarea.setValue('换行')
  const event = pressEnter(textarea, { shiftKey: true })
  expect(w.emitted('start-writing')).toBeUndefined()
  expect(event.defaultPrevented).toBe(false)
})

test('空文本的裸 Enter 不提交', async () => {
  const w = mountBar()
  const textarea = w.find('textarea')
  await textarea.setValue('   ')
  pressEnter(textarea)
  expect(w.emitted('start-writing')).toBeUndefined()
})
