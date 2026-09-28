// 来源 App.vue:402-412 setupConsoleForwarding。
// 纯函数化：拦截 console.log/warn/error/debug，转发给 logFn(level, msg)。
// 调用方负责把 logAppendFrontend 作为 logFn 注入（保持 util 不依赖 tauri-api）。

/**
 * 安全序列化单个 console 参数（F-28）。
 * JSON.stringify 对循环引用（Vue 响应式代理、DOM 节点、事件对象等）会抛
 * `TypeError: Converting circular structure to JSON`；抛出点在**被替换过的
 * console.error 内**，会把"打日志"变成"调用点崩溃"。这里兜住并降级。
 * @param {unknown} value
 * @returns {string|undefined}
 */
export function safeSerializeConsoleArg(value) {
  if (typeof value === 'string') return value
  if (value instanceof Error) return value.message || String(value)
  try {
    return JSON.stringify(value)
  } catch {
    try {
      return String(value)
    } catch {
      return '[Unserializable]'
    }
  }
}

/**
 * 拦截 console.log/warn/error/debug，转发给 logFn。
 * 每条 console 调用：先调原始方法打印到控制台，再调用 logFn(level, msg) 上报后端。
 * @param {(level: string, message: string) => void} logFn - 日志上报回调
 * @returns {void}
 */
export function setupConsoleForwarding(logFn) {
  const levels = { log: 'info', warn: 'warn', error: 'error', debug: 'debug' }
  for (const [method, level] of Object.entries(levels)) {
    const original = console[method]
    console[method] = (...args) => {
      original.apply(console, args)
      const msg = args.map(safeSerializeConsoleArg).join(' ')
      try {
        logFn(level, msg)
      } catch {
        // 上报链路自身失败也不得反噬调用点
      }
    }
  }
}
