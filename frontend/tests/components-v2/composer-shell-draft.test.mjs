// 卡片界面 createChatMessages → 写作输入框草稿：只填入、不发送、不覆盖用户自己的输入。
import { afterEach, expect, test } from 'vitest'
import { nextTick } from 'vue'
import { mount } from '@vue/test-utils'
import ComposerBar from '../../src/design/writing/ComposerBar.vue'
import { offerShellChatDraft } from '../../src/utils/shellChatDraft.js'

let wrapper = null

function mountBar(props = {}) {
  wrapper = mount(ComposerBar, {
    attachTo: document.body,
    props: { writing: false, disabled: false, ...props },
  })
  return wrapper
}

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
})

test('空输入框：填入卡片消息，不触发写作', async () => {
  const w = mountBar()
  expect(offerShellChatDraft('姓名：艾琳\n出身：北境')).toBe('drafted')
  await nextTick()
  expect(w.find('textarea').element.value).toBe('姓名：艾琳\n出身：北境')
  expect(w.emitted('start-writing')).toBeUndefined()

  // 用户点发送才开始写作
  await w.find('button[aria-label="开始写作"]').trigger('click')
  expect(w.emitted('start-writing')).toEqual([['姓名：艾琳\n出身：北境']])
})

test('输入框里有不同的未发送内容：不覆盖', async () => {
  const w = mountBar()
  await w.find('textarea').setValue('我自己写的')
  expect(offerShellChatDraft('卡片的消息')).toBe('draft')
  expect(w.find('textarea').element.value).toBe('我自己写的')
  // 同一段内容重复填入（卡片重试）不算冲突
  await w.find('textarea').setValue('卡片的消息')
  expect(offerShellChatDraft('卡片的消息')).toBe('drafted')
})

test('写作中或禁用：拒绝填入', async () => {
  mountBar({ writing: true })
  expect(offerShellChatDraft('卡片的消息')).toBe('busy')
  wrapper.unmount()
  mountBar({ disabled: true })
  expect(offerShellChatDraft('卡片的消息')).toBe('busy')
})

test('卸载后不再有输入框', () => {
  mountBar()
  wrapper.unmount()
  wrapper = null
  expect(offerShellChatDraft('卡片的消息')).toBe('no-composer')
})
