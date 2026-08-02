import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..')

function readUtf8(filePath) {
  return fs.readFileSync(filePath, 'utf8')
}

function listRustSources(root) {
  const sources = []
  const visit = (directory) => {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const entryPath = path.join(directory, entry.name)
      if (entry.isDirectory()) visit(entryPath)
      else if (entry.isFile() && entry.name.endsWith('.rs')) sources.push(entryPath)
    }
  }
  visit(root)
  return sources.sort()
}

export function extractRegisteredCommands(source) {
  const match = source.match(/tauri::generate_handler!\[([\s\S]*?)\]\s*\)\s*\.run/)
  if (!match) throw new Error('tauri::generate_handler! registration block not found')

  const commands = []
  for (const line of match[1].split(/\r?\n/)) {
    const entryName = line.replace(/\/\/.*$/, '').trim().replace(/,$/, '').trim()
    if (/^(?:[A-Za-z_]\w*::)*[A-Za-z_]\w*$/.test(entryName)) {
      commands.push(entryName.split('::').at(-1))
    }
  }
  return commands
}

export function extractCommandAttributes(source) {
  return [...source.matchAll(/#\[tauri::command(?:\([^\]]*\))?\]/g)].length
}

export function extractFrontendInvokes(source) {
  return [...source.matchAll(/\binvoke\(\s*['"]([^'"]+)['"]/g)].map((match) => match[1])
}

export function extractSqliteReferences(source) {
  const lines = source.split(/\r?\n/)
  const unsupported = []
  lines.forEach((line, index) => {
    if (
      /ensure_json_meta_backend_supported|ensure_typed_patch_backend_supported|sqlite backend skips|unsupported until an atomic SQLite Meta UoW/i.test(
        line,
      )
    ) {
      unsupported.push({ line: index + 1, text: line.trim() })
    }
  })
  return {
    activeFlagReferences: (source.match(/is_sqlite_active\(/g) ?? []).length,
    unsupported,
  }
}

export function collectBaseline(repoRoot = REPO_ROOT) {
  const libPath = path.join(repoRoot, 'crates', 'tauri-app', 'src', 'lib.rs')
  const backendSourceRoot = path.join(repoRoot, 'crates', 'tauri-app', 'src')
  const apiPath = path.join(repoRoot, 'frontend', 'src', 'tauri-api.js')
  const cargoPath = path.join(repoRoot, 'Cargo.toml')
  const libSource = readUtf8(libPath)
  const backendSources = listRustSources(backendSourceRoot).map((filePath) => ({
    filePath,
    source: readUtf8(filePath),
  }))
  const backendSource = backendSources.map(({ source }) => source).join('\n')
  const productionBackendSources = backendSources.filter(
    ({ filePath }) => !path.basename(filePath).startsWith('lib_tests'),
  )
  const productionBackendSource = productionBackendSources
    .map(({ source }) => source)
    .join('\n')
  const apiSource = readUtf8(apiPath)
  const cargoSource = readUtf8(cargoPath)
  const registered = extractRegisteredCommands(libSource)
  const frontend = extractFrontendInvokes(apiSource)
  const registeredSet = new Set(registered)
  const frontendSet = new Set(frontend)
  const sqlite = extractSqliteReferences(productionBackendSource)
  sqlite.facadeFlagReferences = productionBackendSources
    .filter(({ filePath }) => ['sqlite_runtime.rs', 'storage_backend.rs'].includes(path.basename(filePath)))
    .reduce(
      (count, { source }) => count + (source.match(/is_sqlite_active\(/g) ?? []).length,
      0,
    )
  sqlite.applicationFlagReferences =
    sqlite.activeFlagReferences - sqlite.facadeFlagReferences
  sqlite.ambientCharacterStoreReferences = productionBackendSources
    .filter(({ filePath }) => {
      const relative = path.relative(backendSourceRoot, filePath)
      return relative.startsWith(`commands${path.sep}`) || path.basename(filePath) === 'runtime_support.rs'
    })
    .reduce((count, { source }) => count + (source.match(/\bget_store\(\)/g) ?? []).length, 0)
  const selectedWriterConstructorPattern =
    /\b(?:CampaignStore|CharacterStore|TurnStore|CompressJobStore)::new\(/g
  sqlite.facadeSelectedWriterConstructors = productionBackendSources
    .filter(({ filePath }) => path.basename(filePath) === 'storage_backend.rs')
    .reduce(
      (count, { source }) => count + (source.match(selectedWriterConstructorPattern) ?? []).length,
      0,
    )
  sqlite.applicationSelectedWriterConstructors = productionBackendSources
    .filter(({ filePath }) => {
      const relative = path.relative(backendSourceRoot, filePath)
      return relative.startsWith(`commands${path.sep}`) || path.basename(filePath) === 'runtime_support.rs'
    })
    .reduce(
      (count, { source }) => count + (source.match(selectedWriterConstructorPattern) ?? []).length,
      0,
    )
  // Gate 3: `.is_sqlite()` / `.is_json()` method calls are only allowed in the
  // bootstrap file, the facade, the SQLite runtime and the named backend
  // adapter. Every other production source must be zero.
  const backendFlagPattern = /\.is_sqlite\(\)|\.is_json\(\)/g
  const methodFlagFiles = new Map()
  for (const { filePath, source } of productionBackendSources) {
    const count = (source.match(backendFlagPattern) ?? []).length
    if (count > 0) methodFlagFiles.set(path.relative(backendSourceRoot, filePath).replaceAll('\\', '/'), count)
  }
  const methodFlagWhitelist = [
    'lib.rs',
    'storage_backend.rs',
    'sqlite_runtime.rs',
    'backend_workflows.rs',
    // 三审9：启动恢复协调模块（按 is_sqlite 分派 + JSON 路径 facade stores），
    // 与 backend_workflows 同性质——原 lib.rs:243 包装器抽取而来。
    'startup_recovery.rs',
  ]
  sqlite.applicationMethodFlagReferences = [...methodFlagFiles.entries()]
    .filter(([name]) => !methodFlagWhitelist.includes(name))
    .reduce((sum, [, count]) => sum + count, 0)
  sqlite.facadeMethodFlagReferences = [...methodFlagFiles.entries()]
    .filter(([name]) => methodFlagWhitelist.includes(name))
    .reduce((sum, [, count]) => sum + count, 0)
  sqlite.methodFlagReferencesByFile = Object.fromEntries(methodFlagFiles)
  // Direct legacy JSON store accessors (json_character_store / json_campaign_store /
  // json_turn_store / json_compress_job_store) must be confined to the facade +
  // named backend adapter (methodFlagWhitelist). commands/* is NO LONGER a
  // whitelist: the Gate-5 review-followup replaced every command-side
  // `.ok()`-hidden json_character_store access with the backend-neutral
  // `collect_scoped_regex_scripts_for_backend` resolver (stored/source/card/name
  // mapping through the facade). Any reference in commands, card_studio_api,
  // playthrough_lifecycle, runtime_support, … is a backend-policy leak: it makes
  // the path JSON-only and silently breaks under SQLite authority.
  const legacyStoreAccessorPattern =
    /\.json_(?:character|campaign|turn|compress_job)_store\b/g
  const legacyAccessorFiles = new Map()
  for (const { filePath, source } of productionBackendSources) {
    const count = (source.match(legacyStoreAccessorPattern) ?? []).length
    if (count > 0)
      legacyAccessorFiles.set(
        path.relative(backendSourceRoot, filePath).replaceAll('\\', '/'),
        count,
      )
  }
  const legacyAccessorAllowed = (name) => methodFlagWhitelist.includes(name)
  sqlite.applicationLegacyStoreAccessorReferences = [...legacyAccessorFiles.entries()]
    .filter(([name]) => !legacyAccessorAllowed(name))
    .reduce((sum, [, count]) => sum + count, 0)
  sqlite.legacyStoreAccessorReferencesByFile = Object.fromEntries(legacyAccessorFiles)
  const crates = [...cargoSource.matchAll(/^\s*"(crates\/[^"\r\n]+)"\s*,?\s*$/gm)].map(
    (match) => match[1],
  )

  return {
    generatedAt: new Date().toISOString(),
    files: {
      backend: 'crates/tauri-app/src/**/*.rs',
      frontendApi: 'frontend/src/tauri-api.js',
    },
    backend: {
      libLines: libSource.split(/\r?\n/).length,
      commandAttributes: extractCommandAttributes(backendSource),
      registeredCommandCount: registeredSet.size,
      duplicateRegisteredCommands: [...new Set(registered.filter((name, index) => registered.indexOf(name) !== index))].sort(),
      registeredCommands: [...registeredSet].sort(),
    },
    frontend: {
      invokeCount: frontend.length,
      uniqueInvokeCount: frontendSet.size,
      invokedCommands: [...frontendSet].sort(),
      missingBackendCommands: [...frontendSet].filter((name) => !registeredSet.has(name)).sort(),
    },
    workspace: { crateCount: crates.length, crates },
    sqlite,
  }
}

if (process.argv[1]?.endsWith('backend-baseline.mjs')) {
  process.stdout.write(`${JSON.stringify(collectBaseline(), null, 2)}\n`)
}
