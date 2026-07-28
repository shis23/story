import assert from 'node:assert/strict'
import fs from 'node:fs'
import test from 'node:test'
import { collectBaseline } from '../../scripts/architecture/backend-baseline.mjs'

const baseline = collectBaseline()
const libSource = fs.readFileSync(new URL('../../crates/tauri-app/src/lib.rs', import.meta.url), 'utf8')

test('every frontend Tauri invoke is registered by the backend', () => {
  assert.deepEqual(baseline.frontend.missingBackendCommands, [])
})

test('Gate 0 command registration baseline is stable', () => {
  assert.deepEqual(baseline.backend.duplicateRegisteredCommands, [])
  assert.equal(baseline.backend.commandAttributes, 175)
  assert.equal(baseline.backend.registeredCommandCount, 175)
  assert.equal(baseline.frontend.uniqueInvokeCount, 162)
})

test('Gate 0 records the expected SQLite branch surface', () => {
  assert.equal(baseline.workspace.crateCount, 16)
  assert.equal(baseline.sqlite.activeFlagReferences, 68)
  assert.ok(baseline.sqlite.unsupported.length > 0)
})

test('Gate 1 keeps concrete Tauri commands out of lib.rs', () => {
  assert.equal((libSource.match(/#\[tauri::command\]/g) ?? []).length, 0)
})
