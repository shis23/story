import test from 'node:test'
import assert from 'node:assert/strict'

import {
  SHELL_DOC_ORIGIN,
  configureShellDocInvoke,
  registerShellDoc,
  releaseShellDoc,
  shellDocTokenFromUrl,
} from '../src/utils/shellDocUrl.js'

const TOKEN = 'a'.repeat(64)

test('registerShellDoc accepts only a strict host-issued token', async () => {
  configureShellDocInvoke(async (command) => {
    assert.equal(command, 'card_shell_register_doc')
    return TOKEN
  })
  assert.equal(await registerShellDoc('<body>x'), `${SHELL_DOC_ORIGIN}/${TOKEN}`)

  for (const malformed of ['', '../escape', 'A'.repeat(64), 'a'.repeat(63)]) {
    configureShellDocInvoke(async () => malformed)
    await assert.rejects(registerShellDoc('<body>x'), /invalid shell document token/i)
  }
})

test('releaseShellDoc unregisters only URLs from the isolated shell origin', async () => {
  const calls = []
  configureShellDocInvoke(async (command, payload) => {
    calls.push({ command, payload })
    return true
  })

  assert.equal(shellDocTokenFromUrl(`${SHELL_DOC_ORIGIN}/${TOKEN}`), TOKEN)
  assert.equal(await releaseShellDoc(`${SHELL_DOC_ORIGIN}/${TOKEN}`), true)
  assert.deepEqual(calls, [
    { command: 'card_shell_unregister_doc', payload: { token: TOKEN } },
  ])

  assert.equal(shellDocTokenFromUrl(`https://attacker.invalid/${TOKEN}`), null)
  assert.equal(await releaseShellDoc(`https://attacker.invalid/${TOKEN}`), false)
  assert.equal(calls.length, 1)
})
