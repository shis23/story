import assert from 'node:assert/strict'
import test from 'node:test'
import { collectBaseline } from '../../scripts/architecture/backend-baseline.mjs'

const baseline = collectBaseline()

test('every frontend Tauri invoke is registered by the backend', () => {
  assert.deepEqual(baseline.frontend.missingBackendCommands, [])
})

test('Gate 0 command registration baseline is stable', () => {
  assert.deepEqual(baseline.backend.duplicateRegisteredCommands, [])
  assert.equal(baseline.backend.commandAttributes, 156)
  assert.equal(baseline.backend.registeredCommandCount, 175)
  assert.equal(baseline.frontend.uniqueInvokeCount, 162)
})

test('Gate 0 records the expected SQLite branch surface', () => {
  assert.equal(baseline.workspace.crateCount, 16)
  assert.equal(baseline.sqlite.activeFlagReferences, 67)
  assert.ok(baseline.sqlite.unsupported.length > 0)
})
