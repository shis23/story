import test from 'node:test'
import assert from 'node:assert/strict'
import {
  regexPlacementClass,
  regexPlacementLabel,
} from '../src/utils/regexPlacement.js'

test('labels known regex placement codes without changing display text', () => {
  assert.equal(regexPlacementLabel({ placement_codes: [0] }), 'MD显示')
  assert.equal(regexPlacementLabel({ placement_codes: [1] }), '输入')
  assert.equal(regexPlacementLabel({ placement_codes: [2] }), '输出')
  assert.equal(regexPlacementLabel({ placement_codes: [3] }), 'Slash')
  assert.equal(regexPlacementLabel({ placement_codes: [5] }), '世界书')
  assert.equal(regexPlacementLabel({ placement_codes: [6] }), 'Reasoning')
  assert.equal(regexPlacementLabel({ placement_codes: [1, 2, 1, 99] }), '输入/输出/#99')
})

test('keeps known regex placement classes for code-based placements', () => {
  assert.equal(regexPlacementClass({ placement_codes: [1, 2] }), 'bg-accent/10 text-accent')
  assert.equal(regexPlacementClass({ placement_codes: [1] }), 'bg-running/10 text-running')
  assert.equal(regexPlacementClass({ placement_codes: [2] }), 'bg-ok/10 text-ok')
  assert.equal(regexPlacementClass({ placement_codes: [5] }), 'bg-accent/10 text-accent')
  assert.equal(regexPlacementClass({ placement_codes: [0] }), 'bg-warn/10 text-warn')
})

test('falls back to legacy placement labels and classes', () => {
  assert.equal(regexPlacementLabel({ placement: 'input' }), '输入')
  assert.equal(regexPlacementLabel({ placement: 'output' }), '输出')
  assert.equal(regexPlacementLabel({ placement: 'slash_command' }), 'Slash')
  assert.equal(regexPlacementLabel({ placement: 'world_info' }), '世界书')
  assert.equal(regexPlacementLabel({ placement: 'reasoning' }), 'Reasoning')

  assert.equal(regexPlacementClass({ placement: 'input' }), 'bg-running/10 text-running')
  assert.equal(regexPlacementClass({ placement: 'output' }), 'bg-ok/10 text-ok')
  assert.equal(regexPlacementClass({ placement: 'world_info' }), 'bg-accent/10 text-accent')
})

test('falls back safely for unknown and empty placement values', () => {
  assert.equal(regexPlacementLabel({ placement_codes: [42] }), '#42')
  assert.equal(regexPlacementLabel({ placement: 'custom' }), 'custom')
  assert.equal(regexPlacementLabel({ placement: '' }), '未知')
  assert.equal(regexPlacementLabel({}), '未知')
  assert.equal(regexPlacementLabel(null), '未知')

  assert.equal(regexPlacementClass({ placement: 'custom' }), 'bg-warn/10 text-warn')
  assert.equal(regexPlacementClass({}), 'bg-warn/10 text-warn')
  assert.equal(regexPlacementClass(null), 'bg-warn/10 text-warn')
})
