// MVU 变量键记法归一化（后端镜像：crates/domain/src/variables.rs normalize_mvu_key）。
//
// forge 差分实测（2026-07-26）：模型输出键记法不稳定——斜杠记法（/世界/时间）、
// stat_data. 容器前缀、<角色名>/{角色名} 两种模板段并存。后端在解析边界归一
// 新导入产物；前端在匹配/写入点做镜像归一，兜住归一化之前入库的存量产物。

export function normalizeMvuKey(raw) {
  let s = String(raw ?? '').trim()
  // 斜杠记法 → 点记法：仅当以 / 或 stat_data/ 开头（避免误伤含 / 的普通名字）
  if (s.startsWith('/') || s.startsWith('stat_data/')) {
    s = s.replace(/^\/+/, '').replaceAll('/', '.')
  }
  while (s.startsWith('stat_data.')) s = s.slice('stat_data.'.length)
  return s
    .split('.')
    .map((seg) => seg.trim())
    .filter(Boolean)
    .map((seg) =>
      seg.length >= 2 && seg.startsWith('<') && seg.endsWith('>') ? `{${seg.slice(1, -1)}}` : seg,
    )
    .join('.')
}

/**
 * 在变量列表中查目标 key：先精确匹配（零风险快路径），
 * 再按归一化记法匹配（跨记法兜底）。找不到返回 null。
 */
export function findMvuVariable(variables, key) {
  const list = Array.isArray(variables) ? variables : []
  const exact = list.find((v) => v?.key === key)
  if (exact) return exact
  const norm = normalizeMvuKey(key)
  if (!norm) return null
  return list.find((v) => typeof v?.key === 'string' && normalizeMvuKey(v.key) === norm) || null
}
