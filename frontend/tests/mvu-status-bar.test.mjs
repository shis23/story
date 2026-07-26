import test from 'node:test'
import assert from 'node:assert/strict'
import {
  getMvuValue,
  hasMvuFallbackWarning,
  mvuBarColor,
  mvuBarPercent,
  mvuDisplayValue,
  mvuIconFor,
  toMvuNumber,
} from '../src/utils/mvuStatusBarModel.js'

test('resolves native MVU status bar values for bar text tag and icon bindings', () => {
  const variables = [
    { key: 'hp', value: 90 },
    { key: 'mood', value: 'calm' },
    { key: 'state', value: 'focused' },
    { key: 'weather', value: 'rain' },
  ]
  const bar = { variable_key: 'hp', display: { kind: 'bar', max: 120 } }
  const icon = {
    variable_key: 'weather',
    display: { kind: 'icon', mapping: { rain: 'rain-icon', _default: 'sun-icon' } },
  }

  assert.equal(getMvuValue(variables, 'hp'), 90)
  assert.equal(mvuBarPercent(bar, variables), 75)
  assert.equal(mvuBarColor(75), 'bg-ok')
  assert.equal(mvuDisplayValue(variables, 'mood'), 'calm')
  assert.equal(mvuDisplayValue(variables, 'state'), 'focused')
  assert.equal(mvuIconFor(icon, variables, '·'), 'rain-icon')
})

test('clamps bar values and handles missing or invalid variables', () => {
  const high = { variable_key: 'hp', display: { kind: 'bar', max: 100 } }
  const invalid = { variable_key: 'mp', display: { kind: 'bar', max: 100 } }
  const zeroMax = { variable_key: 'hp', display: { kind: 'bar', max: 0 } }

  assert.equal(mvuBarPercent(high, [{ key: 'hp', value: 140 }]), 100)
  assert.equal(mvuBarPercent(high, [{ key: 'hp', value: -5 }]), 0)
  assert.equal(mvuBarPercent(invalid, [{ key: 'mp', value: 'not-a-number' }]), 0)
  assert.equal(mvuBarPercent(zeroMax, [{ key: 'hp', value: 50 }]), 0)
  assert.equal(toMvuNumber('42'), 42)
  assert.equal(toMvuNumber('bad'), 0)
  assert.equal(getMvuValue([], 'missing'), null)
  assert.equal(mvuBarColor(80), 'bg-ok')
  assert.equal(mvuBarColor(40), 'bg-warn')
  assert.equal(mvuBarColor(10), 'bg-err')
})

test('uses icon defaults and exposes JS fallback warning state', () => {
  const binding = {
    variable_key: 'state',
    display: { kind: 'icon', mapping: { happy: ':)', _default: ':|' } },
  }

  assert.equal(mvuIconFor(binding, [{ key: 'state', value: 'happy' }]), ':)')
  assert.equal(mvuIconFor(binding, [{ key: 'state', value: 'unknown' }]), ':|')
  assert.equal(mvuIconFor({ variable_key: 'state', display: { kind: 'icon' } }, [], '·'), '·')
  assert.equal(mvuDisplayValue([], 'missing'), '—')
  assert.equal(hasMvuFallbackWarning(2), true)
  assert.equal(hasMvuFallbackWarning(0), false)
})

test('template-key bindings resolve per instance name', async () => {
  const { getMvuValue, mvuBarPercent } = await import('../src/utils/mvuStatusBarModel.js')
  const vars = [
    { key: '女性角色.小美.好感度', value: 80 },
    { key: '女性角色.阿离.好感度', value: 20 },
  ]
  assert.equal(getMvuValue(vars, '女性角色.{角色名}.好感度', '小美'), 80)
  assert.equal(getMvuValue(vars, '女性角色.{角色名}.好感度', '阿离'), 20)
  // 不传实例名 → 模板键无从展开，查不到
  assert.equal(getMvuValue(vars, '女性角色.{角色名}.好感度'), null)

  const binding = { variable_key: '女性角色.{角色名}.好感度', display: { kind: 'bar', max: 100 } }
  assert.equal(mvuBarPercent(binding, vars, '小美'), 80)
  assert.equal(mvuBarPercent(binding, vars, '阿离'), 20)
})
