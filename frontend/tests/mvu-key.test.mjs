import test from 'node:test'
import assert from 'node:assert/strict'
import { findMvuVariable, normalizeMvuKey } from '../src/utils/mvuKey.js'

test('normalizeMvuKey converges all observed notations to canonical dot form', () => {
  // 点记法（canonical）原样保留
  assert.equal(normalizeMvuKey('主角.属性.力量'), '主角.属性.力量')
  // 斜杠记法（pro 实测输出）→ 点记法
  assert.equal(normalizeMvuKey('/世界/时间'), '世界.时间')
  assert.equal(normalizeMvuKey('/主角/属性/力量'), '主角.属性.力量')
  // stat_data 容器前缀剥离（点/斜杠两种形态）
  assert.equal(normalizeMvuKey('stat_data.主角.好感度'), '主角.好感度')
  assert.equal(normalizeMvuKey('stat_data/主角/好感度'), '主角.好感度')
  // 模板占位符段统一花括号
  assert.equal(normalizeMvuKey('女性角色.<角色名>.好感度'), '女性角色.{角色名}.好感度')
  assert.equal(normalizeMvuKey('女性角色.{角色名}.好感度'), '女性角色.{角色名}.好感度')
  // 空白与空段清理
  assert.equal(normalizeMvuKey('  hp  '), 'hp')
  assert.equal(normalizeMvuKey('主角..hp'), '主角.hp')
  // 不以 / 开头的含斜杠键不转换（可能是普通名字）
  assert.equal(normalizeMvuKey('攻/防'), '攻/防')
  // 边界
  assert.equal(normalizeMvuKey('stat_data'), 'stat_data')
  assert.equal(normalizeMvuKey(null), '')
  assert.equal(normalizeMvuKey(undefined), '')
})

test('findMvuVariable prefers exact match then falls back to normalized match', () => {
  const vars = [
    { key: 'stat_data.hp', value: 80 },
    { key: 'hp', value: 100 },
    { key: '世界.时间', value: '清晨' },
  ]
  // 精确命中优先（即使别的条目归一后也同 key）
  assert.equal(findMvuVariable(vars, 'stat_data.hp')?.value, 80)
  assert.equal(findMvuVariable(vars, 'hp')?.value, 100)
  // 跨记法命中：查询键与存储键记法不同
  assert.equal(findMvuVariable(vars, '/世界/时间')?.value, '清晨')
  assert.equal(findMvuVariable([{ key: 'stat_data.mp', value: 5 }], 'mp')?.value, 5)
  // 找不到
  assert.equal(findMvuVariable(vars, 'nonexistent'), null)
  assert.equal(findMvuVariable(null, 'hp'), null)
})

test('expandMvuTemplateKey substitutes placeholder segments with the instance name', async () => {
  const { expandMvuTemplateKey, findMvuVariableForInstance } = await import('../src/utils/mvuKey.js')

  assert.equal(expandMvuTemplateKey('女性角色.{角色名}.好感度', '小美'), '女性角色.小美.好感度')
  // 无占位符 / 无实例名 → 原样
  assert.equal(expandMvuTemplateKey('主角.hp', '小美'), '主角.hp')
  assert.equal(expandMvuTemplateKey('女性角色.{角色名}.好感度', ''), '女性角色.{角色名}.好感度')
  assert.equal(expandMvuTemplateKey(null, '小美'), '')

  // 实例上下文取值：具体键优先，未命中回退字面模板键
  const vars = [
    { key: '女性角色.小美.好感度', value: 42 },
    { key: '女性角色.{角色名}.好感度', value: 0 },
  ]
  assert.equal(findMvuVariableForInstance(vars, '女性角色.{角色名}.好感度', '小美')?.value, 42)
  // 具体键不存在的实例 → 回退字面模板键（meta_apply 落入实例变量的形态）
  assert.equal(findMvuVariableForInstance(vars, '女性角色.{角色名}.好感度', '阿离')?.value, 0)
  // <> 记法占位符也先归一再展开
  assert.equal(findMvuVariableForInstance(vars, '女性角色.<角色名>.好感度', '小美')?.value, 42)
})
