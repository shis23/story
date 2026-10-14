/**
 * sidecar 可执行文件在哪：插件配置 `bridgeBin` > 随插件装好的平台包 > 仓库 debug 构建产物。
 *
 * 平台包是 esbuild 同款做法：每个平台一个只装可执行文件的 npm 包（`os` / `cpu` 限定），
 * 插件把它们列为可选依赖，包管理器只装当前平台那个。包由 `scripts/pack-bridge.mjs` 打出；
 * 还没有发布，插件 package.json 也还没列可选依赖（列了未发布的包会让安装失败）。
 */

import { existsSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join, resolve } from 'node:path'

/** 可执行文件名。 */
export function bridgeExecutable(platform = process.platform) {
  return platform === 'win32' ? 'storyforge-dsh-bridge.exe' : 'storyforge-dsh-bridge'
}

/** 平台包名（`process.platform` / `process.arch` 取值）。 */
export function bridgePackageName(platform = process.platform, arch = process.arch) {
  return `dsh-plugin-storyforge-bridge-${platform}-${arch}`
}

/** 平台包里可执行文件的相对路径。 */
export function bridgePackageBinPath(platform = process.platform) {
  return join('bin', bridgeExecutable(platform))
}

const pluginRequire = createRequire(import.meta.url)

/**
 * @param {object} options
 * @param {string} [options.configured] 插件配置里的 `bridgeBin`
 * @param {string} options.pluginRoot 插件目录（仓库内时，`../../target/debug` 是构建产物）
 * @param {(request: string) => string} [options.resolvePackage] 解析 `<包名>/package.json`，找不到时抛错
 * @param {(path: string) => boolean} [options.exists]
 * @param {string} [options.platform]
 * @param {string} [options.arch]
 * @returns {{ path: string, source: 'config' | 'package' | 'repo' }}
 */
export function resolveBridgeBin({
  configured,
  pluginRoot,
  resolvePackage = (request) => pluginRequire.resolve(request),
  exists = existsSync,
  platform = process.platform,
  arch = process.arch,
}) {
  if (configured) return { path: configured, source: 'config' }
  let manifest = null
  try {
    manifest = resolvePackage(`${bridgePackageName(platform, arch)}/package.json`)
  } catch {
    // 没装平台包：退回仓库构建产物。
  }
  if (manifest) {
    const packaged = join(dirname(manifest), bridgePackageBinPath(platform))
    if (exists(packaged)) return { path: packaged, source: 'package' }
  }
  return { path: resolve(pluginRoot, '../../target/debug', bridgeExecutable(platform)), source: 'repo' }
}
