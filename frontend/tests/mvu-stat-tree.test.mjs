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

// M-28c(a)：叶子先写、整体标量后写，不得整棵覆盖子树（两种顺序同结果）
test('subtree wins over a scalar whole-value write in both orders', () => {
  const leafFirst = buildMvuStatDataTree([
    { key: '主角.生命值', value: 80 },
    { key: '主角', value: 'scalar' },
  ])
  const scalarFirst = buildMvuStatDataTree([
    { key: '主角', value: 'scalar' },
    { key: '主角.生命值', value: 80 },
  ])

  assert.deepEqual(leafFirst, { 主角: { 生命值: 80 } })
  assert.deepEqual(leafFirst, scalarFirst)

  // 整体值是对象时按字段递归合并（两种顺序同结果，不整体替换）
  const objectFirst = buildMvuStatDataTree([
    { key: '主角', value: { 力量: 12 } },
    { key: '主角.生命值', value: 80 },
  ])
  const objectLast = buildMvuStatDataTree([
    { key: '主角.生命值', value: 80 },
    { key: '主角', value: { 力量: 12 } },
  ])
  assert.deepEqual(objectFirst, { 主角: { 力量: 12, 生命值: 80 } })
  assert.deepEqual(objectLast, objectFirst)

  // 同深度标量冲突仍是「后到者覆盖」
  assert.deepEqual(
    buildMvuStatDataTree([{ key: '主角.生命值', value: 80 }, { key: '主角.生命值', value: 70 }]),
    { 主角: { 生命值: 70 } },
  )
})

// M-28c(b)：blocked 死分支删除后，中间段被标量占据时仍让位给子树（不静默丢变量）
test('a mid-path scalar never blocks the deeper writes', () => {
  const tree = buildMvuStatDataTree([
    { key: '主角', value: 'scalar' },
    { key: '主角.属性.力量', value: 12 },
    { key: '主角.生命值', value: 80 },
  ])

  assert.deepEqual(tree, { 主角: { 属性: { 力量: 12 }, 生命值: 80 } })
})
