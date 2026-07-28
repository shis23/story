import { analyzeEsmModuleSource } from './tavernHelperScripts.js'

function resolveHttpModuleUrl(value, base) {
  let parsed
  try {
    parsed = base ? new URL(value, base) : new URL(value)
  } catch {
    throw new Error('invalid module URL: ' + value)
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
    throw new Error('unsupported module URL protocol: ' + parsed.protocol)
  }
  return parsed.href
}

function isResolvableSpecifier(specifier) {
  return (
    specifier.startsWith('http://') ||
    specifier.startsWith('https://') ||
    specifier.startsWith('.') ||
    specifier.startsWith('/')
  )
}

/**
 * Publish a remote ES module graph behind shell-protocol URLs. No card source
 * is evaluated in the host while imports are discovered and rewritten.
 */
export async function registerShellModuleGraph({
  entryUrl,
  entrySource,
  preamble = '',
  fetchText,
  registerModule,
  releaseModule,
  maxDepth = 12,
}) {
  if (typeof entrySource !== 'string') {
    throw new Error('entry module source must be a string')
  }
  if (typeof fetchText !== 'function' || typeof registerModule !== 'function') {
    throw new Error('module graph fetch/register callbacks are required')
  }

  const normalizedEntry = resolveHttpModuleUrl(entryUrl)
  const context = {
    cache: new Map(),
    visiting: new Set(),
    sources: new Map([[normalizedEntry, entrySource]]),
    leases: [],
  }

  const load = async (moduleUrl, depth = 0) => {
    if (depth > maxDepth) {
      throw new Error('module graph too deep: ' + moduleUrl)
    }
    const normalized = resolveHttpModuleUrl(moduleUrl)
    if (context.cache.has(normalized)) return context.cache.get(normalized)
    if (context.visiting.has(normalized)) {
      throw new Error('cyclic module graph is not supported: ' + normalized)
    }
    context.visiting.add(normalized)

    try {
      const source = context.sources.has(normalized)
        ? context.sources.get(normalized)
        : await fetchText(normalized)
      if (typeof source !== 'string') {
        throw new Error('module response is not text: ' + normalized)
      }

      const analysis = analyzeEsmModuleSource(source)
      const replacements = []
      for (const imported of analysis.imports) {
        if (!imported.specifier) continue
        if (!isResolvableSpecifier(imported.specifier)) {
          throw new Error(
            'unsupported bare module specifier: ' + imported.specifier,
          )
        }
        const dependencyUrl = resolveHttpModuleUrl(
          imported.specifier,
          normalized,
        )
        const shellUrl = await load(dependencyUrl, depth + 1)
        replacements.push({
          start: imported.start,
          end: imported.end,
          value: shellUrl,
        })
      }

      let rewritten = source
      for (const replacement of replacements.sort(
        (left, right) => right.start - left.start,
      )) {
        rewritten =
          rewritten.slice(0, replacement.start) +
          replacement.value +
          rewritten.slice(replacement.end)
      }
      if (depth === 0 && preamble) rewritten = preamble + rewritten

      const shellUrl = await registerModule(rewritten)
      context.leases.push(shellUrl)
      context.cache.set(normalized, shellUrl)
      return shellUrl
    } finally {
      context.visiting.delete(normalized)
    }
  }

  try {
    const url = await load(normalizedEntry)
    return { url, leases: context.leases.slice() }
  } catch (error) {
    if (typeof releaseModule === 'function') {
      await Promise.all(
        context.leases.map((url) =>
          Promise.resolve(releaseModule(url)).catch(() => {}),
        ),
      )
    }
    throw error
  }
}
