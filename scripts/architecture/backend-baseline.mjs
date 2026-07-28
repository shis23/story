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
  const backendSources = listRustSources(backendSourceRoot).map((filePath) => readUtf8(filePath))
  const backendSource = backendSources.join('\n')
  const apiSource = readUtf8(apiPath)
  const cargoSource = readUtf8(cargoPath)
  const registered = extractRegisteredCommands(libSource)
  const frontend = extractFrontendInvokes(apiSource)
  const registeredSet = new Set(registered)
  const frontendSet = new Set(frontend)
  const sqlite = extractSqliteReferences(backendSource)
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
