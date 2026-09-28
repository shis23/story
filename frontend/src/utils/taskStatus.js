/**
 * 任务状态是 serde 外部标签枚举（`crates/domain/src/story_task.rs`）：
 * `TaskStatus::LikelyCompleted { confidence: f32 }` 序列化为
 * `{"likely_completed":{"confidence":0.5}}`（F-08）。旧扁平形状
 * `{"likely_completed":0.5}` 仍兼容，避免历史数据/夹具渲染成 NaN%。
 */
export function likelyCompletedConfidence(status) {
  const value = status?.likely_completed
  if (typeof value === 'number') return value
  if (value && typeof value === 'object' && typeof value.confidence === 'number') return value.confidence
  return null
}

export function isLikelyCompleted(status) {
  return likelyCompletedConfidence(status) != null
}

export function taskStatusText(status) {
  if (typeof status === 'string') return status
  const confidence = likelyCompletedConfidence(status)
  if (confidence != null) return `可能完成 (${Math.round(confidence * 100)}%)`
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
