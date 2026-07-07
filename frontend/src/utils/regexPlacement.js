const PLACEMENT_CODE_LABELS = new Map([
  [0, 'MD显示'],
  [1, '输入'],
  [2, '输出'],
  [3, 'Slash'],
  [5, '世界书'],
  [6, 'Reasoning'],
])

const LEGACY_PLACEMENT_LABELS = new Map([
  ['input', '输入'],
  ['output', '输出'],
  ['slash_command', 'Slash'],
  ['world_info', '世界书'],
  ['reasoning', 'Reasoning'],
])

const WARN_CLASS = 'bg-warn/10 text-warn'

export function regexPlacementLabel(regex) {
  const codes = Array.isArray(regex?.placement_codes) ? regex.placement_codes : []
  if (codes.length === 0) {
    return LEGACY_PLACEMENT_LABELS.get(regex?.placement) || regex?.placement || '未知'
  }

  const labels = codes.map((code) => PLACEMENT_CODE_LABELS.get(code) || `#${code}`)
  return [...new Set(labels)].join('/')
}

export function regexPlacementClass(regex) {
  const codes = Array.isArray(regex?.placement_codes) ? regex.placement_codes : []
  if (codes.length > 0) {
    const hasInput = codes.includes(1)
    const hasOutput = codes.includes(2)
    const hasWorldInfo = codes.includes(5)
    if (hasInput && hasOutput) return 'bg-accent/10 text-accent'
    if (hasInput) return 'bg-running/10 text-running'
    if (hasOutput) return 'bg-ok/10 text-ok'
    if (hasWorldInfo) return 'bg-accent/10 text-accent'
    return WARN_CLASS
  }

  if (regex?.placement === 'input') return 'bg-running/10 text-running'
  if (regex?.placement === 'output') return 'bg-ok/10 text-ok'
  if (regex?.placement === 'world_info') return 'bg-accent/10 text-accent'
  return WARN_CLASS
}
