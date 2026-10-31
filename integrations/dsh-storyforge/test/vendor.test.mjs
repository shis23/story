// `lib/vendor/card-shell.mjs` 是桌面前端模块的打包产物：重新打包与入库文件逐字相同，
// 否则提示重新生成（桌面源码改了而产物没跟上，DSH 里的卡片界面就和桌面不一致）。
// `lib/vendor/purify.es.mjs` 是 DOMPurify 的直拷贝（DSH 卡片 HTML 净化，与桌面同一防线）：
// 两处副本必须同版本逐字一致，且不低于 critical XSS 修复线（3.4.12 存在
// GHSA-55q2-fjhq-7xh7 等 5 条 2026 公告）。
// 运行：node --test integrations/dsh-storyforge/test/*.test.mjs

import assert from 'node:assert/strict'
import { existsSync, readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import { test } from 'node:test'

import { VENDOR_FILE, buildVendor, loadEsbuild } from '../scripts/build-vendor.mjs'

const pluginRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const PURIFY_VENDOR = resolve(pluginRoot, 'lib/vendor/purify.es.mjs')
const PURIFY_INSTALLED = resolve(pluginRoot, '../../frontend/node_modules/dompurify/dist/purify.es.mjs')
/** DOMPurify critical XSS（GHSA-55q2-fjhq-7xh7 等）修复版本下限。 */
const PURIFY_MIN_VERSION = [3, 4, 17]

let esbuild = null
try {
  esbuild = loadEsbuild()
} catch {
  esbuild = null
}

test('vendor/card-shell.mjs 与桌面源码同步', { skip: esbuild ? false : '没装 frontend 依赖（cd frontend && npm ci）' }, async () => {
  const fresh = await buildVendor(esbuild)
  const committed = readFileSync(VENDOR_FILE, 'utf8').replace(/\r\n/g, '\n')
  assert.ok(
    committed === fresh.replace(/\r\n/g, '\n'),
    '产物过期：运行 node integrations/dsh-storyforge/scripts/build-vendor.mjs 重新生成',
  )
})

function purifyVersion(file) {
  const banner = readFileSync(file, 'utf8').slice(0, 400)
  const m = banner.match(/DOMPurify (\d+)\.(\d+)\.(\d+)/)
  assert.ok(m, `无法从 ${file} 头部解析 DOMPurify 版本号`)
  return [Number(m[1]), Number(m[2]), Number(m[3])]
}

function atLeast(version, floor) {
  for (let i = 0; i < 3; i += 1) {
    if (version[i] !== floor[i]) return version[i] > floor[i]
  }
  return true
}

test('vendor/purify.es.mjs 与 frontend 安装版逐字一致', { skip: existsSync(PURIFY_INSTALLED) ? false : '没装 frontend 依赖（cd frontend && npm ci）' }, () => {
  const vendor = readFileSync(PURIFY_VENDOR, 'utf8')
  const installed = readFileSync(PURIFY_INSTALLED, 'utf8')
  assert.ok(
    vendor === installed,
    'DOMPurify 双拷贝漂移：升级 frontend 依赖后必须同步重建 vendor——\n' +
    '  cp frontend/node_modules/dompurify/dist/purify.es.mjs integrations/dsh-storyforge/lib/vendor/purify.es.mjs',
  )
})

test('DOMPurify 版本不低于 critical XSS 修复线', () => {
  const vendor = purifyVersion(PURIFY_VENDOR)
  assert.ok(
    atLeast(vendor, PURIFY_MIN_VERSION),
    `vendor DOMPurify ${vendor.join('.')} 低于 ${PURIFY_MIN_VERSION.join('.')}（critical XSS，GHSA-55q2-fjhq-7xh7 等 5 条 2026 公告），两处需一起升级`,
  )
  if (existsSync(PURIFY_INSTALLED)) {
    const installed = purifyVersion(PURIFY_INSTALLED)
    assert.ok(atLeast(installed, PURIFY_MIN_VERSION), `frontend 安装的 DOMPurify ${installed.join('.')} 低于修复线`)
  }
})
