// 本地打包：当前平台的 sidecar 平台包 + 插件本体，各出一个 npm tarball（只打包，不发布）。
//
//   node integrations/dsh-storyforge/scripts/pack-bridge.mjs [--skip-build] [--out <目录>]
//
// - 先 `cargo build --release -p storyforge-dsh-bridge`（--skip-build 时直接用现有的 release 产物）。
// - 平台包 `dsh-plugin-storyforge-bridge-<platform>-<arch>`：只有 `bin/` 下的可执行文件，
//   package.json 用 `os` / `cpu` 限定平台，版本跟插件一致。插件按 lib/bridge-bin.js 的顺序找它。
// - 插件本体按 package.json 的 `files` 打包（运行时文件，不含测试与打包脚本）。
// - 默认输出到仓库 `target/dsh-dist/`（target 已被 git 忽略）。

import { execFileSync, execSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { bridgeExecutable, bridgePackageBinPath, bridgePackageName } from '../lib/bridge-bin.js'

const pluginRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = resolve(pluginRoot, '../..')
const args = process.argv.slice(2)
const skipBuild = args.includes('--skip-build')
const outIndex = args.indexOf('--out')
const outDir = resolve(outIndex >= 0 && args[outIndex + 1] ? args[outIndex + 1] : join(repoRoot, 'target', 'dsh-dist'))

const plugin = JSON.parse(readFileSync(join(pluginRoot, 'package.json'), 'utf8'))

/** 给 shell 的路径参数加引号（cmd 用双引号，sh 用单引号）。 */
function shellQuote(value) {
  if (process.platform === 'win32') {
    if (value.includes('"')) throw new Error(`路径里不能有双引号：${value}`)
    return `"${value}"`
  }
  return `'${value.replaceAll("'", "'\\''")}'`
}

/** `npm pack` 一个目录，返回 tarball 的绝对路径。 */
function pack(dir) {
  // npm 在 Windows 上是 .cmd，只能经 shell 启动；整条命令自己拼、自己加引号。
  const output = execSync(`npm pack --json --pack-destination ${shellQuote(outDir)}`, { cwd: dir, encoding: 'utf8' })
  const [info] = JSON.parse(output)
  return { file: join(outDir, info.filename), entries: info.files.map((entry) => entry.path) }
}

function sha256(file) {
  return createHash('sha256').update(readFileSync(file)).digest('hex')
}

if (!skipBuild) {
  console.log('cargo build --release -p storyforge-dsh-bridge')
  execFileSync('cargo', ['build', '--release', '-p', 'storyforge-dsh-bridge'], { cwd: repoRoot, stdio: 'inherit' })
}
const binary = join(repoRoot, 'target', 'release', bridgeExecutable())
if (!existsSync(binary)) {
  console.error(`找不到 release 产物：${binary}（去掉 --skip-build 先构建）`)
  process.exit(1)
}

mkdirSync(outDir, { recursive: true })
const name = bridgePackageName()
const packageDir = join(outDir, name)
rmSync(packageDir, { recursive: true, force: true })
mkdirSync(join(packageDir, 'bin'), { recursive: true })
copyFileSync(binary, join(packageDir, bridgePackageBinPath()))
writeFileSync(
  join(packageDir, 'package.json'),
  `${JSON.stringify(
    {
      name,
      version: plugin.version,
      description: `StoryForge DSH 插件的 sidecar（${process.platform}-${process.arch}）`,
      license: plugin.license,
      os: [process.platform],
      cpu: [process.arch],
      files: ['bin/'],
    },
    null,
    2,
  )}\n`,
)
writeFileSync(
  join(packageDir, 'README.md'),
  `# ${name}\n\n${plugin.name} 的 sidecar 可执行文件（${process.platform}-${process.arch}）。插件自动找到它，不需要单独使用。\n`,
)

const bridgeTarball = pack(packageDir)
const pluginTarball = pack(pluginRoot)
for (const { file, entries } of [bridgeTarball, pluginTarball]) {
  console.log(`${file}\n  ${(statSync(file).size / 1024 / 1024).toFixed(1)} MiB  sha256 ${sha256(file)}  ${entries.length} 个文件`)
}
