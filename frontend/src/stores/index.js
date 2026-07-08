import { createPinia } from 'pinia'

// 全局 Pinia 实例工厂。
// main.js 切换到 AppV2 时:createApp(AppV2).use(createPinia()).mount('#app')
// 阶段 1 各 store 写好后,这里会 re-export 4 个 useXxxStore。
export const pinia = createPinia()

// 阶段 1 完成后取消注释:
// export { useCampaignStore } from './campaign.js'
// export { useWritingStore } from './writing.js'
// export { usePluginStore } from './plugin.js'
// export { useUiStore } from './ui.js'
