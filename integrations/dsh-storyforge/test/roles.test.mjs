// 底层调用的管线角色识别：取系统提示词里最早出现的身份句。
import assert from 'node:assert/strict'
import test from 'node:test'

import { classifyCall } from '../lib/roles.js'

const system = (content) => [{ role: 'system', content }, { role: 'user', content: '……' }]

test('recognizes each pipeline role by its identity sentence', () => {
  assert.equal(classifyCall(system('你是写作导演。用户给你写作意图，你要派子 Agent 演出：你是角色 X 时……')), '导演')
  assert.equal(classifyCall(system('你是编剧。收集所有子 Agent 的表演')), '编剧')
  assert.equal(classifyCall(system('你是执笔者，不是故事里的任何角色。')), '执笔')
  assert.equal(classifyCall(system('你是对手戏场记定稿员。')), '定稿')
  assert.equal(classifyCall(system('你是本轮剧情总结助手（round summary）')), '摘要')
  assert.equal(classifyCall(system('你是后处理助手（postprocess）')), '后处理')
  assert.equal(classifyCall(system('你是卡内角色识别助手（character recognition）')), '角色识别')
  assert.equal(classifyCall(system('你是顺序剧组演员 苏沁，正在执行第 1/3 拍。')), '角色 · 苏沁')
  assert.equal(classifyCall([{ role: 'user', content: '你好' }]), '其它')
  assert.equal(classifyCall(undefined), '其它')
})

test('subagent name comes from the last identity line, skipping the unfilled template', () => {
  const prompt = '你是角色 {name}。根据导演给你的任务演出。\n\n你是角色 王曼离。\n\n## 你的角色设定\n你是编剧社的社长。'
  assert.equal(classifyCall(system(prompt)), '角色 · 王曼离')
})

test('prepended prompt modules do not hide the identity', () => {
  const prompt = '【写作风格】简洁克制。\n\n你是后处理助手（postprocess）。更新角色知识。'
  assert.equal(classifyCall(system(prompt)), '后处理')
})
