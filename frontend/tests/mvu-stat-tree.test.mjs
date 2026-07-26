import test from 'node:test'
import assert from 'node:assert/strict'
import { buildMvuStatDataTree } from '../src/utils/mvuStatTree.js'

test('builds a nested stat tree from dot-path campaign variables', () => {
  const tree = buildMvuStatDataTree([
    { key: '主角.生命值', value: 80 },
    { key: '主角.属性.力量', value: 12 },
    { key: '世界.时间', value: '黄昏' },
    { key: 'story_clock', value: 'day 3' },
  ])

  assert.deepEqual(tree, {
    主角: { 生命值: 80, 属性: { 力量: 12 } },
    世界: { 时间: '黄昏' },
    story_clock: 'day 3',
  })
})

test('normalizes legacy key notations and skips internal namespaces', () => {
  const tree = buildMvuStatDataTree([
    { key: 'stat_data.主角.好感度', value: 30 },
    { key: '/世界/天气', value: '雨' },
    { key: '__storyforge_card_shell_variables', value: { 'character:current': {} } },
    { key: '', value: 'junk' },
    { key: null, value: 'junk' },
  ])

  assert.deepEqual(tree, {
    主角: { 好感度: 30 },
    世界: { 天气: '雨' },
  })
})

test('scalar leaf yields to a deeper subtree on path conflict', () => {
  const tree = buildMvuStatDataTree([
    { key: '主角', value: 'scalar' },
    { key: '主角.生命值', value: 80 },
  ])
  assert.deepEqual(tree, { 主角: { 生命值: 80 } })

  // undefined 值落成 null，不产出 undefined 洞
  const holes = buildMvuStatDataTree([{ key: 'flag' }])
  assert.deepEqual(holes, { flag: null })
})
