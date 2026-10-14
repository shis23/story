// 生成 `lib/vendor/card-shell.mjs`：把 `shell/entry.mjs` 列出的桌面前端模块打成一个 ES 模块。
//
//   node integrations/dsh-storyforge/scripts/build-vendor.mjs
//
// esbuild 用桌面前端已装的那份（`frontend/node_modules`，先 `npm ci`）。产物入库；
// `test/vendor.test.mjs` 用同一个函数重新打包并逐字比对，桌面源码一改就会提示重新生成。

import { writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const here = path.dirname(fileURLToPath(import.meta.url))
const pluginDir = path.resolve(here, '..')
const repoRoot = path.resolve(pluginDir, '../..')

export const VENDOR_FILE = path.join(pluginDir, 'lib', 'vendor', 'card-shell.mjs')

/** 桌面前端的 esbuild；没装依赖时抛错（调用方决定跳过还是失败）。 */
export function loadEsbuild() {
  return createRequire(path.join(repoRoot, 'frontend', 'package.json'))('esbuild')
}

/** 打包，返回产物文本（不落盘）。路径注释相对仓库根，产物与机器无关。 */
export async function buildVendor(esbuild = loadEsbuild()) {
  const result = await esbuild.build({
    absWorkingDir: repoRoot,
    entryPoints: ['integrations/dsh-storyforge/shell/entry.mjs'],
    bundle: true,
    format: 'esm',
    platform: 'neutral',
    target: 'es2022',
    charset: 'utf8',
    legalComments: 'none',
    write: false,
    banner: {
      js: [
        '// 生成文件，不要手改：node integrations/dsh-storyforge/scripts/build-vendor.mjs',
        `// 来源：integrations/dsh-storyforge/shell/entry.mjs 列出的桌面前端模块（esbuild ${esbuild.version}）。`,
      ].join('\n'),
    },
  })
  return result.outputFiles[0].text
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const text = await buildVendor()
  writeFileSync(VENDOR_FILE, text)
  console.log(`wrote ${path.relative(repoRoot, VENDOR_FILE)} (${text.length} chars)`)
}
