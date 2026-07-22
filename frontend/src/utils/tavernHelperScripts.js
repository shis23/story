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
