import test from 'node:test'
import assert from 'node:assert/strict'
import {
  DETAIL_SUB_TABS,
  SUB_TAB_REFS,
  subTabRefKey,
  refreshSubTab,
} from '../src/utils/campaignTabRefresh.js'

test('DETAIL_SUB_TABS contains all expected sub-tab names', () => {
  assert.deepEqual(DETAIL_SUB_TABS, [
    'instances',
    'variables',
    'knowledge',
    'worldinfo',
    'tasks',
    'summaries',
  ])
})

test('SUB_TAB_REFS maps each sub-tab to a ref key', () => {
  assert.equal(SUB_TAB_REFS.instances, 'instancesTabRef')
  assert.equal(SUB_TAB_REFS.variables, 'variablesTabRef')
  assert.equal(SUB_TAB_REFS.knowledge, 'knowledgeTabRef')
  assert.equal(SUB_TAB_REFS.worldinfo, 'worldInfoTabRef')
  assert.equal(SUB_TAB_REFS.tasks, 'tasksTabRef')
  assert.equal(SUB_TAB_REFS.summaries, 'summariesTabRef')
})

test('subTabRefKey returns known ref keys', () => {
  for (const tab of DETAIL_SUB_TABS) {
    const refKey = subTabRefKey(tab)
    assert.ok(refKey, `sub-tab "${tab}" should have a ref key`)
    assert.ok(refKey.endsWith('TabRef'), `ref key for "${tab}" should end with TabRef`)
  }
})

test('subTabRefKey returns null for unknown sub-tab names', () => {
  assert.equal(subTabRefKey('nonexistent'), null)
  assert.equal(subTabRefKey(''), null)
  assert.equal(subTabRefKey(null), null)
  assert.equal(subTabRefKey(undefined), null)
})

test('refreshSubTab calls the correct ref and returns refreshed:true', () => {
  let calledKey = null
  const tabRefs = {
    instancesTabRef: { refresh: () => { calledKey = 'instancesTabRef' } },
    knowledgeTabRef: { refresh: () => { calledKey = 'knowledgeTabRef' } },
  }

  const result = refreshSubTab('knowledge', tabRefs)
  assert.equal(result.refreshed, true)
  assert.equal(result.refKey, 'knowledgeTabRef')
  assert.equal(calledKey, 'knowledgeTabRef')
})

test('refreshSubTab returns refreshed:false when sub-tab is not in refs map', () => {
  const result = refreshSubTab('nonexistent', { instancesTabRef: { refresh: () => {} } })
  assert.equal(result.refreshed, false)
  assert.equal(result.refKey, null)
})

test('refreshSubTab returns refreshed:false when ref key exists but ref is null', () => {
  const result = refreshSubTab('instances', { instancesTabRef: null })
  assert.equal(result.refreshed, false)
  assert.equal(result.refKey, 'instancesTabRef')
})

test('refreshSubTab returns refreshed:false when ref has no refresh function', () => {
  const result = refreshSubTab('instances', { instancesTabRef: {} })
  assert.equal(result.refreshed, false)
  assert.equal(result.refKey, 'instancesTabRef')
})

test('refreshSubTab returns refreshed:false when tabRefs is empty', () => {
  const result = refreshSubTab('instances', {})
  assert.equal(result.refreshed, false)
  assert.equal(result.refKey, 'instancesTabRef')
})

test('refreshSubTab only calls the matching sub-tab, not others', () => {
  const called = []
  const tabRefs = {
    instancesTabRef: { refresh: () => { called.push('instances') } },
    knowledgeTabRef: { refresh: () => { called.push('knowledge') } },
    tasksTabRef: { refresh: () => { called.push('tasks') } },
  }

  refreshSubTab('tasks', tabRefs)
  assert.deepEqual(called, ['tasks'])
})
