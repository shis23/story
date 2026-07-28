import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'

import {
  SHELL_DOC_ORIGIN,
  configureShellDocInvoke,
  registerShellModule,
  releaseShellModule,
  shellModuleTokenFromUrl,
  shellModuleUrlForToken,
} from '../src/utils/shellDocUrl.js'

const TOKEN = 'b'.repeat(64)

test('registerShellModule publishes source behind a strict host-issued token', async () => {
  const calls = []
  configureShellDocInvoke(async (command, payload) => {
    calls.push({ command, payload })
    return TOKEN
  })

  assert.equal(
    await registerShellModule('export const answer = 42'),
    `${SHELL_DOC_ORIGIN}/module/${TOKEN}`,
  )
  assert.deepEqual(calls, [{
    command: 'card_shell_register_module',
    payload: { source: 'export const answer = 42' },
  }])

  configureShellDocInvoke(async () => '../escape')
  await assert.rejects(
    registerShellModule('export default 1'),
    /invalid shell module token/i,
  )
})

test('shellModuleUrlForToken rejects paths and uppercase tokens', () => {
  assert.equal(
    shellModuleUrlForToken(TOKEN),
    `${SHELL_DOC_ORIGIN}/module/${TOKEN}`,
  )
  for (const malformed of ['../escape', 'B'.repeat(64), 'b'.repeat(63)]) {
    assert.throws(() => shellModuleUrlForToken(malformed), /invalid shell module token/i)
  }
})

test('releaseShellModule unregisters only a strict module URL', async () => {
  const calls = []
  configureShellDocInvoke(async (command, payload) => {
    calls.push({ command, payload })
    return true
  })
  const url = shellModuleUrlForToken(TOKEN)

  assert.equal(shellModuleTokenFromUrl(url), TOKEN)
  assert.equal(await releaseShellModule(url), true)
  assert.deepEqual(calls, [{
    command: 'card_shell_unregister_doc',
    payload: { token: TOKEN },
  }])
  assert.equal(shellModuleTokenFromUrl(`${SHELL_DOC_ORIGIN}/${TOKEN}`), null)
  assert.equal(await releaseShellModule(`${SHELL_DOC_ORIGIN}/module/../escape`), false)
  assert.equal(calls.length, 1)
})

test('CardShell and TavernHelper register module source instead of importing blob:null or large data URLs', () => {
  const cardShell = fs.readFileSync(
    new URL('../src/components/CardShellHost.vue', import.meta.url),
    'utf8',
  )
  const tavernHelper = fs.readFileSync(
    new URL('../src/components/TavernHelperRuntime.vue', import.meta.url),
    'utf8',
  )

  for (const source of [cardShell, tavernHelper]) {
    assert.match(source, /registerShellModule/)
    assert.match(source, /registerShellModuleGraph/)
    assert.match(source, /ask\('register_module', \{ source: code \}\)/)
    assert.match(source, /ask\('release_module', \{ url: url \}\)/)
    assert.match(source, /active(?:Shell)?ModuleLeases\.add\(url\)/)
    assert.doesNotMatch(source, /moduleSourceToDataUrlAssignment/)
    assert.doesNotMatch(source, /import\(blobUrl\)/)
  }

  assert.match(cardShell, /prepare_inline_module/)
  assert.doesNotMatch(cardShell, /__sfShellImportSpecRe/)
})
