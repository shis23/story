// 来源 App.vue:402-412 setupConsoleForwarding。
// 纯函数化：拦截 console.log/warn/error/debug，转发给 logFn(level, msg)。
// 调用方负责把 logAppendFrontend 作为 logFn 注入（保持 util 不依赖 tauri-api）。

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
      const msg = args.map((a) => (typeof a === 'string' ? a : JSON.stringify(a))).join(' ')
      logFn(level, msg)
    }
  }
}
