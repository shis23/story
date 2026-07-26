import test from 'node:test'
import assert from 'node:assert/strict'
import {
  buildCampaignMvuStatusSections,
  buildInstanceMvuStatusBarProps,
  findInstanceDefinition,
  mergeVariablesForMvuStatus,
  shouldApplyInstanceMvuLoad,
} from '../src/utils/campaignMvuStatusBar.js'

const card = {
  source_character_id: 'source-card',
  character_definitions: [
    { id: 'def-a', name: 'A' },
    { id: 'def-b', name: 'B' },
  ],
}

const translationDetail = {
  source_character_id: 'source-card',
  translation: {
    ui_bindings: [
      { element: 'hp-bar', variable_key: 'hp', display: { kind: 'bar', max: 100 } },
    ],
    fallback_fragments: [{ code: 'customRender()' }],
  },
}

test('builds MVU status bar props for a campaign instance bound to the card definition', () => {
  const variables = [{ key: 'hp', value: 42 }]
  const props = buildInstanceMvuStatusBarProps({
    instance: { id: 'inst-a', definition_id: 'def-a' },
    card,
    translationDetail,
    variables,
  })

  assert.deepEqual(findInstanceDefinition(card, { definition_id: 'def-a' }), { id: 'def-a', name: 'A' })
  assert.equal(props.definitionId, 'def-a')
  assert.equal(props.sourceCharacterId, 'source-card')
  assert.equal(props.fallbackCount, 1)
  assert.deepEqual(props.uiBindings, translationDetail.translation.ui_bindings)
  assert.deepEqual(props.variables, variables)
})

test('skips MVU status bar props for temporary or orphan campaign instances', () => {
  assert.equal(buildInstanceMvuStatusBarProps({
    instance: { id: 'temp', definition_id: null },
    card,
    translationDetail,
    variables: [],
  }), null)

  assert.equal(buildInstanceMvuStatusBarProps({
    instance: { id: 'orphan', definition_id: 'missing-def' },
    card,
    translationDetail,
    variables: [],
  }), null)
})

test('skips MVU status bar props when translation has nothing renderable', () => {
  assert.equal(buildInstanceMvuStatusBarProps({
    instance: { id: 'inst-a', definition_id: 'def-a' },
    card,
    translationDetail: { translation: { ui_bindings: [], fallback_fragments: [] } },
    variables: [],
  }), null)
})

test('merges campaign variables under instance variables with instance winning on key clash', () => {
  const merged = mergeVariablesForMvuStatus(
    [{ key: 'weather', value: '雨' }, { key: 'hp', value: 1 }],
    [{ key: 'hp', value: 42 }],
  )
  assert.deepEqual(merged, [
    { key: 'weather', value: '雨' },
    { key: 'hp', value: 42 },
  ])
  assert.deepEqual(mergeVariablesForMvuStatus(null, undefined), [])
})

test('builds writing-surface sections for card-bound instances only', () => {
  const sections = buildCampaignMvuStatusSections({
    card,
    translationDetail,
    instances: [
      { id: 'inst-a', name: '江离', definition_id: 'def-a', variables: [{ key: 'hp', value: 42 }] },
      { id: 'temp', name: '路人', definition_id: null, variables: [] },
      { id: 'orphan', name: '孤儿', definition_id: 'missing-def', variables: [] },
    ],
    campaignVariables: [{ key: 'weather', value: '雨' }],
  })

  assert.equal(sections.length, 1)
  assert.equal(sections[0].instanceId, 'inst-a')
  assert.equal(sections[0].instanceName, '江离')
  assert.deepEqual(sections[0].mvuState.variables, [
    { key: 'weather', value: '雨' },
    { key: 'hp', value: 42 },
  ])
  assert.deepEqual(sections[0].mvuState.uiBindings, translationDetail.translation.ui_bindings)
})

test('builds empty section list without instances or renderable translation', () => {
  assert.deepEqual(buildCampaignMvuStatusSections({
    card,
    translationDetail,
    instances: null,
    campaignVariables: [],
  }), [])

  assert.deepEqual(buildCampaignMvuStatusSections({
    card,
    translationDetail: { translation: { ui_bindings: [], fallback_fragments: [] } },
    instances: [{ id: 'inst-a', name: 'A', definition_id: 'def-a', variables: [] }],
    campaignVariables: [],
  }), [])
})

test('applies async instance MVU loads only for the current expanded instance and token', () => {
  assert.equal(shouldApplyInstanceMvuLoad({
    expandedInstanceId: 'inst-b',
    instanceId: 'inst-b',
    token: 2,
    currentToken: 2,
  }), true)

  assert.equal(shouldApplyInstanceMvuLoad({
    expandedInstanceId: 'inst-b',
    instanceId: 'inst-a',
    token: 1,
    currentToken: 2,
  }), false)

  assert.equal(shouldApplyInstanceMvuLoad({
    expandedInstanceId: null,
    instanceId: 'inst-a',
    token: 1,
    currentToken: 1,
  }), false)
})
