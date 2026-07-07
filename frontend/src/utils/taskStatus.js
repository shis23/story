export function taskStatusText(status) {
  if (typeof status === 'string') return status
  if (status?.likely_completed != null) return `可能完成 (${Math.round(status.likely_completed * 100)}%)`
  return JSON.stringify(status)
}

export function taskStatusClass(status) {
  const s = typeof status === 'string' ? status : ''
  if (s === 'pending') return 'bg-wait/10 text-wait'
  if (s === 'active') return 'bg-running/10 text-running'
  if (s === 'completed') return 'bg-ok/10 text-ok'
  if (s === 'abandoned') return 'bg-ink-soft/10 text-ink-soft'
  return 'bg-warn/10 text-warn'
}
