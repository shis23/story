// sidecar 可执行文件的查找顺序：配置 > 平台包 > 仓库 debug 产物。

import assert from 'node:assert/strict'
import { join, resolve } from 'node:path'
import { test } from 'node:test'

import { bridgeExecutable, bridgePackageBinPath, bridgePackageName, resolveBridgeBin } from '../lib/bridge-bin.js'

const pluginRoot = resolve('/repo/integrations/dsh-storyforge')
const missing = () => {
  throw Object.assign(new Error('not found'), { code: 'MODULE_NOT_FOUND' })
}

test('平台包名与可执行文件名按平台区分', () => {
  assert.equal(bridgePackageName('win32', 'x64'), 'dsh-plugin-storyforge-bridge-win32-x64')
  assert.equal(bridgePackageName('linux', 'arm64'), 'dsh-plugin-storyforge-bridge-linux-arm64')
  assert.equal(bridgeExecutable('win32'), 'storyforge-dsh-bridge.exe')
  assert.equal(bridgeExecutable('darwin'), 'storyforge-dsh-bridge')
  assert.equal(bridgePackageBinPath('linux'), join('bin', 'storyforge-dsh-bridge'))
})

test('配置了 bridgeBin 就用它，不再找平台包', () => {
  let asked = false
  const found = resolveBridgeBin({
    configured: 'D:/tools/bridge.exe',
    pluginRoot,
    resolvePackage: () => {
      asked = true
      return '/x/package.json'
    },
  })
  assert.deepEqual(found, { path: 'D:/tools/bridge.exe', source: 'config' })
  assert.equal(asked, false)
})

test('装了当前平台的包就用包里的可执行文件', () => {
  const manifest = resolve('/node_modules/dsh-plugin-storyforge-bridge-linux-x64/package.json')
  const requests = []
  const found = resolveBridgeBin({
    pluginRoot,
    platform: 'linux',
    arch: 'x64',
    resolvePackage: (request) => {
      requests.push(request)
      return manifest
    },
    exists: () => true,
  })
  assert.deepEqual(requests, ['dsh-plugin-storyforge-bridge-linux-x64/package.json'])
  assert.deepEqual(found, {
    path: join(resolve('/node_modules/dsh-plugin-storyforge-bridge-linux-x64'), 'bin', 'storyforge-dsh-bridge'),
    source: 'package',
  })
})

test('没装平台包、或包里缺可执行文件时退回仓库 debug 产物', () => {
  const repoBin = resolve(pluginRoot, '../../target/debug', 'storyforge-dsh-bridge.exe')
  assert.deepEqual(resolveBridgeBin({ pluginRoot, platform: 'win32', arch: 'x64', resolvePackage: missing }), {
    path: repoBin,
    source: 'repo',
  })
  const empty = resolveBridgeBin({
    pluginRoot,
    platform: 'win32',
    arch: 'x64',
    resolvePackage: () => resolve('/node_modules/p/package.json'),
    exists: () => false,
  })
  assert.deepEqual(empty, { path: repoBin, source: 'repo' })
})
