/**
 * L7-A：重型 TH 应用识别与拆分。
 *
 * 卡 manifest 里的大体量内联 TH 脚本（卿卿 bgm/图鉴/cg 类，57-99K）是
 * 完整的可视应用——在隐藏 0×0 运行时里执行等于白跑（UI 无处渲染、
 * autoplay 无手势永远被拦）。这里按字节量把它们从常规逻辑脚本
 * （MagVarUpdate 等，通常 <30K）里拆出来，交给写作面 HeavyShellDock
 * 单独「展开即挂载」。
 *
 * 阈值取 30K：已知重型应用最小 57K、逻辑脚本最大 ~20K，中间留一倍余量。
 * 判定只看 inline_js（remote_url 模块另有信任面；inline_html 有消息内挂载路径）。
 */

export const HEAVY_TH_INLINE_BYTES = 30_000

/** 单壳判定：tavern_helper_module + inline_js 且体量达到阈值。 */
export function isHeavyThShell(shell) {
  if (!shell || shell.kind !== 'tavern_helper_module') return false
  const inline = shell.entry?.inline_js
  if (!inline || typeof inline !== 'object') return false
  const bytes = inline.byte_len || (typeof inline.js === 'string' ? inline.js.length : 0)
  return bytes >= HEAVY_TH_INLINE_BYTES
}

/**
 * 把 manifest.shells 拆成 { light, heavy }（原对象引用，不复制）。
 * light 给隐藏运行时（逻辑脚本照常自动跑），heavy 给可见 dock（手势挂载）。
 */
export function splitHeavyThShells(shells) {
  const light = []
  const heavy = []
  for (const shell of Array.isArray(shells) ? shells : []) {
    if (isHeavyThShell(shell)) heavy.push(shell)
    else light.push(shell)
  }
  return { light, heavy }
}

/** dock chip 显示用：k 单位体量标签。 */
export function heavyShellSizeLabel(shell) {
  const inline = shell?.entry?.inline_js
  const bytes = inline?.byte_len || (typeof inline?.js === 'string' ? inline.js.length : 0)
  if (!bytes) return ''
  return `${Math.round(bytes / 1024)}K`
}
