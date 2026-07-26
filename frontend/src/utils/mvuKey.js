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
 * 模板占位符段展开：`女性角色.{角色名}.好感度` + 实例名「小美」→
 * `女性角色.小美.好感度`。占位符段整段替换为实例名；无占位符或无实例名
 * 时原样返回。调用方应先 normalizeMvuKey（<xxx> 统一成 {xxx}）。
 */
export function expandMvuTemplateKey(key, instanceName) {
  const name = String(instanceName ?? '').trim()
  const raw = String(key ?? '')
  if (!name || !raw.includes('{')) return raw
  return raw
    .split('.')
    .map((seg) => (/^\{.+\}$/.test(seg.trim()) ? name : seg))
    .join('.')
}

/**
 * 实例上下文取变量：模板键先按实例名展开查具体键（精确→归一化），
 * 未命中回退字面模板键（meta_apply 会把模板键原样落进实例变量）。
 */
export function findMvuVariableForInstance(variables, key, instanceName) {
  const expanded = expandMvuTemplateKey(normalizeMvuKey(key), instanceName)
  if (expanded && expanded !== String(key ?? '')) {
    const hit = findMvuVariable(variables, expanded)
    if (hit) return hit
  }
  return findMvuVariable(variables, key)
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
