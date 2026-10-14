// `lib/vendor/card-shell.mjs` 是桌面前端模块的打包产物：重新打包与入库文件逐字相同，
// 否则提示重新生成（桌面源码改了而产物没跟上，DSH 里的卡片界面就和桌面不一致）。
// 运行：node --test integrations/dsh-storyforge/test/*.test.mjs

import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'

import { VENDOR_FILE, buildVendor, loadEsbuild } from '../scripts/build-vendor.mjs'

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
