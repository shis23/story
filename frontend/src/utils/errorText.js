/**
 * errorText — 把后端/插件错误对象转成用户可读文案。
 *
 * Tauri 命令错误是结构化 DTO（crates/tauri-app/src/error.rs）：
 *   { type: 'validation' | 'llm' | 'pipeline' | ..., message: string }
 * 直接模板字符串拼接会得到 "[object Object]"（2026-08-31 验收：待采纳
 * 屏障拒绝的提示条只显示 [object Object]）。
 */
export function errorText(e) {
  if (e == null) return '未知错误'
  if (typeof e === 'string') return e
  if (typeof e === 'object') {
    // JS Error 实例：保留 "TypeError: xxx" 形式（含错误类别）
    if (e instanceof Error) {
      const s = e.toString()
      if (s && s !== '[object Object]') return s
    }
    // Tauri 命令错误 DTO：{ type, message }
    if (typeof e.message === 'string' && e.message) return e.message
    if (typeof e.toString === 'function') {
      const s = e.toString()
      if (s && s !== '[object Object]') return s
    }
  }
  return String(e)
}
