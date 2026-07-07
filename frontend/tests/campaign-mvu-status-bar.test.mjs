import test from 'node:test'
import assert from 'node:assert/strict'
import {
  buildInstanceMvuStatusBarProps,
  findInstanceDefinition,
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
