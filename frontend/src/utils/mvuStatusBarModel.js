export function getMvuValue(variables, key) {
  const found = (variables || []).find(item => item.key === key)
  return found ? found.value : null
}

export function toMvuNumber(value) {
  if (value == null) return 0
  const number = Number(value)
  return Number.isNaN(number) ? 0 : number
}

export function mvuBarPercent(binding, variables) {
  const value = toMvuNumber(getMvuValue(variables, binding?.variable_key))
  const max = binding?.display?.max ?? 100
  if (max <= 0) return 0
  return Math.max(0, Math.min(100, (value / max) * 100))
}

export function mvuBarColor(percent) {
  if (percent > 50) return 'bg-ok'
  if (percent > 25) return 'bg-warn'
  return 'bg-err'
}

export function mvuIconFor(binding, variables, fallbackIcon = '-') {
  const value = getMvuValue(variables, binding?.variable_key)
  const mapping = binding?.display?.mapping || {}
  const key = String(value)
  return mapping[key] || mapping._default || fallbackIcon
}

export function hasMvuFallbackWarning(fallbackCount) {
  return Number(fallbackCount) > 0
}

export function mvuDisplayValue(variables, key, fallback = '—') {
  return getMvuValue(variables, key) ?? fallback
}
