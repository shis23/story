/**
 * Normalize CardShellManifest.shells into ordered tavern_helper executables.
 *
 * Domain CardShellEntry is serde externally tagged + rename_all=snake_case:
 *   { "remote_url": { "url": "..." } }
 *   { "inline_js": { "js": "..." } }
 */

export function parseShellEntry(entry) {
  if (!entry || typeof entry !== 'object') return { kind: 'unknown' }
  if (entry.remote_url && typeof entry.remote_url === 'object') {
    return { kind: 'remote_url', url: entry.remote_url.url || null }
  }
  if (entry.inline_js && typeof entry.inline_js === 'object') {
    return {
      kind: 'inline_js',
      js: entry.inline_js.js || null,
      deferred: !!entry.inline_js.deferred,
      byteLen: entry.inline_js.byte_len || 0,
    }
  }
  if (entry.inline_html && typeof entry.inline_html === 'object') {
    return { kind: 'inline_html', html: entry.inline_html.html || null }
  }
  // PascalCase external-tag fallback
  if (entry.RemoteUrl && typeof entry.RemoteUrl === 'object') {
    return { kind: 'remote_url', url: entry.RemoteUrl.url || null }
  }
  if (entry.InlineJs && typeof entry.InlineJs === 'object') {
    return { kind: 'inline_js', js: entry.InlineJs.js || null }
  }
  return { kind: 'unknown' }
}

/**
 * @param {Array<object>} shells
 * @returns {Array<{index:number,label:string,kind:'remote_url'|'inline_js',url?:string,js?:string,deps:string[]}>}
 */
export function orderedTavernHelperFromShells(shells) {
  const list = Array.isArray(shells) ? shells : []
  const out = []
  let i = 0
  for (const s of list) {
    if (!s || s.kind !== 'tavern_helper_module') continue
    const parsed = parseShellEntry(s.entry)
    const label = s.label || `th#${i}`
    const deps = Array.isArray(s.deps) ? s.deps.slice() : []
    const buttons = Array.isArray(s.buttons) ? s.buttons.filter(Boolean) : []
    if (parsed.kind === 'remote_url' && parsed.url) {
      out.push({ index: i++, label, kind: 'remote_url', url: parsed.url, deps, buttons })
    } else if (parsed.kind === 'inline_js' && (parsed.js || parsed.deferred)) {
      out.push({
        index: i++,
        label,
        kind: 'inline_js',
        js: parsed.js || '',
        deferred: !!parsed.deferred,
        deps,
        buttons,
      })
    }
  }
  return out
}


/** Flatten unique visible TH buttons from ordered scripts (preserve first-seen order). */
export function collectVisibleThButtons(scripts) {
  const seen = new Set()
  const out = []
  for (const s of Array.isArray(scripts) ? scripts : []) {
    for (const name of s.buttons || []) {
      const n = String(name || '').trim()
      if (!n || seen.has(n)) continue
      seen.add(n)
      out.push({ name: n, scriptLabel: s.label, scriptIndex: s.index })
    }
  }
  return out
}

/**
 * SillyTavern stores remote card scripts as a single side-effect import.
 * This is a pointer to a classic script bundle, not an instruction to execute
 * the bundle itself with ES-module semantics.
 */
export function extractBareImportTarget(source) {
  if (typeof source !== 'string') return null
  const match = source.match(
    /^\s*import\s*(['"])([^'"\r\n]+)\1\s*;?\s*$/,
  )
  return match ? match[2] : null
}

/**
 * Parse real ESM syntax without mistaking strings/comments for imports.
 * `start`/`end` delimit only the specifier text, so callers can safely rewrite
 * dependencies while preserving the surrounding import statement.
 */
export function analyzeEsmModuleSource(source) {
  if (typeof source !== 'string') {
    throw new Error('module source must be a string')
  }

  const imports = []
  let isModule = false
  let statementStart = true
  let braceDepth = 0
  let index = 0

  const isIdentifierStart = (ch) => /[A-Za-z_$]/.test(ch || '')
  const isIdentifierPart = (ch) => /[A-Za-z0-9_$]/.test(ch || '')

  const skipQuoted = (start, quote) => {
    let cursor = start + 1
    while (cursor < source.length) {
      if (source[cursor] === '\\') {
        cursor += 2
        continue
      }
      if (source[cursor] === quote) return cursor + 1
      cursor += 1
    }
    return source.length
  }

  const skipLineComment = (start) => {
    const end = source.indexOf('\n', start + 2)
    return end < 0 ? source.length : end
  }

  const skipBlockComment = (start) => {
    const end = source.indexOf('*/', start + 2)
    return end < 0 ? source.length : end + 2
  }

  const skipSpaceAndComments = (start) => {
    let cursor = start
    while (cursor < source.length) {
      if (/\s/.test(source[cursor])) {
        cursor += 1
        continue
      }
      if (source.startsWith('//', cursor)) {
        cursor = skipLineComment(cursor)
        continue
      }
      if (source.startsWith('/*', cursor)) {
        cursor = skipBlockComment(cursor)
        continue
      }
      break
    }
    return cursor
  }

  const statementEnd = (start) => {
    let cursor = start
    let localDepth = 0
    while (cursor < source.length) {
      const ch = source[cursor]
      if (ch === "'" || ch === '"' || ch === '`') {
        cursor = skipQuoted(cursor, ch)
        continue
      }
      if (source.startsWith('//', cursor)) {
        cursor = skipLineComment(cursor)
        continue
      }
      if (source.startsWith('/*', cursor)) {
        cursor = skipBlockComment(cursor)
        continue
      }
      if (ch === '(' || ch === '[' || ch === '{') localDepth += 1
      if (ch === ')' || ch === ']' || ch === '}') {
        localDepth = Math.max(0, localDepth - 1)
      }
      if (ch === ';' && localDepth === 0) return cursor
      cursor += 1
    }
    return source.length
  }

  const pushSpecifier = (start, end, dynamic = false) => {
    imports.push({
      specifier: source.slice(start, end),
      start,
      end,
      dynamic,
    })
  }

  while (index < source.length) {
    const ch = source[index]
    if (/\s/.test(ch)) {
      if ((ch === '\n' || ch === '\r') && braceDepth === 0) {
        statementStart = true
      }
      index += 1
      continue
    }
    if (source.startsWith('//', index)) {
      index = skipLineComment(index)
      continue
    }
    if (source.startsWith('/*', index)) {
      index = skipBlockComment(index)
      continue
    }
    if (ch === "'" || ch === '"' || ch === '`') {
      index = skipQuoted(index, ch)
      statementStart = false
      continue
    }
    if (ch === '{') {
      braceDepth += 1
      statementStart = false
      index += 1
      continue
    }
    if (ch === '}') {
      braceDepth = Math.max(0, braceDepth - 1)
      statementStart = braceDepth === 0
      index += 1
      continue
    }
    if (ch === ';') {
      statementStart = braceDepth === 0
      index += 1
      continue
    }
    if (!isIdentifierStart(ch)) {
      statementStart = false
      index += 1
      continue
    }

    const tokenStart = index
    index += 1
    while (isIdentifierPart(source[index])) index += 1
    const token = source.slice(tokenStart, index)
    const next = skipSpaceAndComments(index)
    if (token === 'import' && source[next] === '(') {
      isModule = true
      const argument = skipSpaceAndComments(next + 1)
      if (source[argument] === "'" || source[argument] === '"') {
        const quoteEnd = skipQuoted(argument, source[argument]) - 1
        pushSpecifier(argument + 1, quoteEnd, true)
      }
      statementStart = false
      continue
    }
    if (
      braceDepth !== 0 ||
      !statementStart ||
      (token !== 'import' && token !== 'export')
    ) {
      statementStart = false
      continue
    }

    isModule = true
    const end = statementEnd(index)
    if (token === 'import' && (source[next] === "'" || source[next] === '"')) {
      const quoteEnd = skipQuoted(next, source[next]) - 1
      pushSpecifier(next + 1, quoteEnd)
    } else {
      const statement = source.slice(index, end)
      const from = /\bfrom\s*(['"])([^'"\r\n]+)\1/.exec(statement)
      if (from) {
        const relativeStart = from.index + from[0].indexOf(from[2])
        pushSpecifier(index + relativeStart, index + relativeStart + from[2].length)
      }
    }
    index = end < source.length ? end + 1 : end
    statementStart = true
  }

  return {
    isModule,
    imports,
  }
}

function resolveHttpScriptUrl(value, base) {
  let url
  try {
    url = base ? new URL(value, base) : new URL(value)
  } catch {
    throw new Error('invalid card script URL: ' + value)
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') {
    throw new Error('unsupported card script URL protocol: ' + url.protocol)
  }
  return url.href
}

/**
 * Fetch a TavernHelper script as classic JavaScript. Import-only wrappers are
 * followed as references, while the final bundle is returned unchanged.
 */
export async function fetchClassicScriptSource(entryUrl, fetchText, maxHops = 8) {
  if (typeof fetchText !== 'function') {
    throw new Error('fetchText must be a function')
  }

  const seen = new Set()
  let current = resolveHttpScriptUrl(entryUrl)
  for (let hop = 0; hop < maxHops; hop += 1) {
    if (seen.has(current)) {
      throw new Error('card script import wrapper cycle: ' + current)
    }
    seen.add(current)

    const source = await fetchText(current)
    if (typeof source !== 'string') {
      throw new Error('card script response is not text: ' + current)
    }
    const target = extractBareImportTarget(source)
    if (!target) return source
    current = resolveHttpScriptUrl(target, current)
  }

  throw new Error('card script import wrapper chain is too deep')
}
