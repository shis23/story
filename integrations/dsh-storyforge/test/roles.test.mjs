// 底层调用的管线角色识别：取系统提示词里最早出现的身份句。
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

import { classifyCall, IDENTITY_SENTENCES, ROLE_GROUPS, roleGroup } from '../lib/roles.js'

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

test('call labels map to the groups that can get their own model', () => {
  assert.equal(roleGroup('导演'), 'director')
  assert.equal(roleGroup('角色 · 苏沁'), 'actor')
  assert.equal(roleGroup('角色'), 'actor')
  for (const label of ['执笔', '编剧', '定稿']) assert.equal(roleGroup(label), 'prose', label)
  for (const label of ['摘要', '后处理', '纪要压缩']) assert.equal(roleGroup(label), 'settle', label)
  for (const label of ['角色识别', 'MVU 分析']) assert.equal(roleGroup(label), 'analysis', label)
  // 「角色识别」以「角色」开头，但不是角色表演。
  assert.notEqual(roleGroup('角色识别'), 'actor')
  assert.equal(roleGroup('其它'), null)
  assert.equal(roleGroup(undefined), null)
  for (const group of Object.values(ROLE_GROUPS)) assert.ok(group.label && group.hint)
})

// 按角色选模型靠这些身份句识别调用：Rust 提示词改了措辞，这里要先红，而不是静默退回底层模型。
const IDENTITY_SOURCES = {
  你是写作导演: 'crates/app-pipeline/src/lib.rs',
  你是编剧: 'crates/app-pipeline/src/lib.rs',
  你是执笔者: 'crates/app-pipeline/src/lib.rs',
  你是对手戏场记定稿员: 'crates/app-pipeline/src/lib.rs',
  你是本轮剧情总结助手: 'crates/app-agent/src/prompts/summarizer.rs',
  你是后处理助手: 'crates/app-agent/src/prompts/postprocess.rs',
  你是卡内角色识别助手: 'crates/app-agent/src/prompts/character_extractor.rs',
  你是剧情纪要压缩助手: 'crates/app-agent/src/prompts/chronicle_compressor.rs',
  你是卡内状态栏分析助手: 'crates/app-meta/src/prompts/mvu_analyzer.rs',
  '你是角色 ': 'crates/app-agent/src/runtime.rs',
  '你是顺序剧组演员 ': 'crates/app-pipeline/src/sequential_crew.rs',
}

test('every identity sentence is still in the Rust prompt it comes from', () => {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), '../../..')
  assert.deepEqual(Object.keys(IDENTITY_SOURCES).sort(), [...IDENTITY_SENTENCES].sort())
  for (const [sentence, file] of Object.entries(IDENTITY_SOURCES)) {
    const source = readFileSync(join(root, file), 'utf8')
    // 只看测试模块之前的生产代码。
    const production = source.split(/^#\[cfg\(test\)\]/m)[0]
    assert.ok(production.includes(sentence), `${file} 里找不到「${sentence}」`)
  }
})
