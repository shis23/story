import { createPinia } from 'pinia'

// 全局 Pinia 实例工厂。
// main.js 切换到 AppV2 时:createApp(AppV2).use(createPinia()).mount('#app')
export const pinia = createPinia()

// 阶段 1:4 个 store re-export
export { useCampaignStore } from './campaign.js'
export { useWritingStore } from './writing.js'
export { usePluginStore } from './plugin.js'
export { useUiStore } from './ui.js'
