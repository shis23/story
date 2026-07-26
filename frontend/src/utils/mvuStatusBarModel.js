import { findMvuVariableForInstance } from './mvuKey.js'

// instanceName 可选：实例节上下文中模板键（女性角色.{角色名}.好感度）
// 按实例名展开后查具体键，未命中回退字面键。
export function getMvuValue(variables, key, instanceName = '') {
  const found = findMvuVariableForInstance(variables, key, instanceName)
  return found ? found.value : null
}

export function toMvuNumber(value) {
  if (value == null) return 0
  const number = Number(value)
  return Number.isNaN(number) ? 0 : number
}

export function mvuBarPercent(binding, variables, instanceName = '') {
  const value = toMvuNumber(getMvuValue(variables, binding?.variable_key, instanceName))
  const max = binding?.display?.max ?? 100
  if (max <= 0) return 0
  return Math.max(0, Math.min(100, (value / max) * 100))
}

export function mvuBarColor(percent) {
  if (percent > 50) return 'bg-ok'
  if (percent > 25) return 'bg-warn'
  return 'bg-err'
}

export function mvuIconFor(binding, variables, fallbackIcon = '-', instanceName = '') {
  const value = getMvuValue(variables, binding?.variable_key, instanceName)
  const mapping = binding?.display?.mapping || {}
  const key = String(value)
  return mapping[key] || mapping._default || fallbackIcon
}

export function hasMvuFallbackWarning(fallbackCount) {
  return Number(fallbackCount) > 0
}

export function mvuDisplayValue(variables, key, fallback = '—', instanceName = '') {
  return getMvuValue(variables, key, instanceName) ?? fallback
}
