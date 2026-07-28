import assert from 'node:assert/strict'
import fs from 'node:fs'
import test from 'node:test'
import { collectBaseline } from '../../scripts/architecture/backend-baseline.mjs'

const baseline = collectBaseline()
const libSource = fs.readFileSync(new URL('../../crates/tauri-app/src/lib.rs', import.meta.url), 'utf8')
const rootTestSource = fs.readFileSync(new URL('../../crates/tauri-app/src/lib_tests.rs', import.meta.url), 'utf8')
const commandSnapshot = JSON.parse(
  fs.readFileSync(new URL('./fixtures/tauri-registered-commands.snapshot.json', import.meta.url), 'utf8'),
)

test('every frontend Tauri invoke is registered by the backend', () => {
  assert.deepEqual(baseline.frontend.missingBackendCommands, [])
})

test('Gate 0 command registration baseline is stable', () => {
  assert.deepEqual(baseline.backend.duplicateRegisteredCommands, [])
  assert.equal(baseline.backend.commandAttributes, 175)
  assert.equal(baseline.backend.registeredCommandCount, 175)
  assert.equal(baseline.frontend.uniqueInvokeCount, 162)
})

test('Gate 0 command registration matches the complete ordered snapshot', () => {
  assert.deepEqual(baseline.backend.registeredCommands, commandSnapshot)
})

test('Gate 0 records the expected SQLite branch surface', () => {
  assert.equal(baseline.workspace.crateCount, 16)
  assert.equal(baseline.sqlite.activeFlagReferences, 68)
  assert.ok(baseline.sqlite.unsupported.length > 0)
})

test('Gate 1 keeps concrete Tauri commands out of lib.rs', () => {
  assert.equal((libSource.match(/#\[tauri::command\]/g) ?? []).length, 0)
})

test('Gate 1 keeps moved domain implementations out of lib.rs', () => {
  const movedImplementations = [
    'regenerate_impl',
    'configure_embedder_impl',
    'configure_embedder_async',
    'configure_embedder_with_secret_store_async',
    'get_embed_config_impl',
    'archive_conversation_impl',
    'archivable_messages_from_conversation',
    'load_archive_snapshot',
    'run_archive_with_watermark',
    'auto_archive_if_needed',
    'mutation_is_receipt_reviewable',
    'receipt_items_from_batch',
    'retain_selected_receipt_mutations',
    'active_turn_receipt_from_record',
    'get_active_turn_receipt_impl',
    'apply_turn_receipt_selection',
    'postprocess_present_characters',
    'retry_active_turn_postprocess_impl',
    'active_turn_quality_from_record',
    'get_active_turn_quality_impl',
  ]
  for (const name of movedImplementations) {
    assert.doesNotMatch(libSource, new RegExp(`\\bfn\\s+${name}\\b`), `implementation ${name} leaked into lib.rs`)
    assert.doesNotMatch(libSource, new RegExp(`crate::${name}\\b`), `wrapper still routes through root ${name}`)
  }
})

test('Gate 1 keeps lib.rs within the bootstrap boundary', () => {
  assert.ok(libSource.split(/\r?\n/).length <= 2500)
})

test('Gate 1 keeps domain tests out of the root test module', () => {
  assert.equal((rootTestSource.match(/#\[(?:tokio::)?test\]/g) ?? []).length, 0)
  assert.ok(rootTestSource.split(/\r?\n/).length <= 260)

  const domainTestFiles = [
    'lib_tests_connections.rs',
    'lib_tests_conversations.rs',
    'lib_tests_turns.rs',
    'lib_tests_writing.rs',
    'lib_tests_writing_regenerate.rs',
    'lib_tests_startup.rs',
    'lib_tests_import_export.rs',
    'lib_tests_campaigns.rs',
    'lib_tests_meta.rs',
    'lib_tests_diagnostics.rs',
  ]
  for (const file of domainTestFiles) {
    const source = fs.readFileSync(new URL('../../crates/tauri-app/src/' + file, import.meta.url), 'utf8')
    assert.match(source, /#\[(?:tokio::)?test\]/, file + ' must own executable domain tests')
  }
})
