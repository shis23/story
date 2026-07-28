import test from 'node:test'
import assert from 'node:assert/strict'

import { registerShellModuleGraph } from '../src/utils/shellModuleGraph.js'

test('registerShellModuleGraph rewrites minified static and literal dynamic imports', async () => {
  const fetched = new Map([
    ['https://cdn.example.test/dep.js', 'export const dep = 1;'],
    ['https://cdn.example.test/lazy.js', 'export default 2;'],
  ])
  const registered = []
  const result = await registerShellModuleGraph({
    entryUrl: 'https://cdn.example.test/entry.js',
    entrySource:
      "import{dep}from'./dep.js';const lazy=()=>import('./lazy.js');export{dep};",
    preamble: 'const host = globalThis;\n',
    fetchText: async (url) => fetched.get(url),
    registerModule: async (source) => {
      registered.push(source)
      return `http://storyforge-shell.localhost/module/${String(registered.length).padStart(64, 'a')}`
    },
  })

  assert.equal(result.leases.length, 3)
  assert.equal(result.url, result.leases[2])
  assert.match(registered[2], /^const host = globalThis;/)
  assert.match(registered[2], new RegExp(result.leases[0].replaceAll('/', '\\/')))
  assert.match(registered[2], new RegExp(result.leases[1].replaceAll('/', '\\/')))
  assert.doesNotMatch(registered[2], /\.\/dep\.js|\.\/lazy\.js/)
})

test('registerShellModuleGraph releases partial registrations after failure', async () => {
  const released = []
  await assert.rejects(
    registerShellModuleGraph({
      entryUrl: 'https://cdn.example.test/entry.js',
      entrySource:
        "import './ok.js';import './missing.js';",
      fetchText: async (url) => {
        if (url.endsWith('/ok.js')) return 'export const ok = true;'
        throw new Error('missing dependency')
      },
      registerModule: async () =>
        `http://storyforge-shell.localhost/module/${'b'.repeat(64)}`,
      releaseModule: async (url) => {
        released.push(url)
      },
    }),
    /missing dependency/,
  )
  assert.deepEqual(released, [
    `http://storyforge-shell.localhost/module/${'b'.repeat(64)}`,
  ])
})
