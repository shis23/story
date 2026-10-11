/** 宿主半边与 agent 作用域插件共用的常量。 */

/** StoryForge 适配器注册的 provider 路由。 */
export const PROVIDER = 'storyforge'

/** StoryForge 会话固定使用的 model id（不出现在任何模型选择器里）。 */
export const MODEL = 'pipeline'

/** StoryForge 专属 agent 预设 id。 */
export const PRESET = 'storyforge'

/** 浏览器半边访问的受保护路由前缀（connection.fetch，即 /api 通道）。 */
export const API_BASE = '/api/storyforge'

export const log = (...args) => console.log('[storyforge]', ...args)
