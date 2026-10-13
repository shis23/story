/** 宿主半边与 agent 作用域插件共用的常量与进程内状态。 */

/** StoryForge 适配器注册的 provider 路由。 */
export const PROVIDER = 'storyforge'

/** StoryForge 会话固定使用的 model id（不出现在任何模型选择器里）。 */
export const MODEL = 'pipeline'

/** StoryForge 专属 agent 预设 id。 */
export const PRESET = 'storyforge'

/** 插件创建的 StoryForge 会话 id 前缀（分叉出的子会话仍是 DSH 的 `session-` 前缀）。 */
export const SESSION_PREFIX = 'storyforge-'

/** 开场消息的来源 kind：适配器据此识别"开场"而不是用户行动。 */
export const OPENING_SOURCE_KIND = 'storyforge'

/** 浏览器半边访问的受保护路由前缀（connection.fetch，即 /api 通道）。 */
export const API_BASE = '/api/storyforge'

export const log = (...args) => console.log('[storyforge]', ...args)

const MAX_EVENTS = 100

/**
 * 生命周期事件（agent 创建 / 分叉）。agent 作用域插件与宿主半边是同一进程里的同一个
 * ES 模块实例，因此可以直接共享。
 */
export const lifecycleEvents = []

export function recordLifecycle(event) {
  lifecycleEvents.unshift({ at: new Date().toISOString(), ...event })
  lifecycleEvents.length = Math.min(lifecycleEvents.length, MAX_EVENTS)
}

/** 宿主半边登记的会话创建回调：agent 作用域插件发现分叉子会话时通知它建立映射。 */
let forkObserver = null

export function setForkObserver(observer) {
  forkObserver = observer
  return () => {
    if (forkObserver === observer) forkObserver = null
  }
}

export function notifyFork(childId, parentId) {
  forkObserver?.(childId, parentId)
}
