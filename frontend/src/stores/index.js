// Pinia 实例由 main.js 自建（createApp(AppV2).use(createPinia())），
// 此处仅作 store re-export 汇总入口，不再额外导出无人消费的全局实例。

// 阶段 1:4 个 store re-export
export { useCampaignStore } from './campaign.js'
export { useWritingStore } from './writing.js'
export { usePluginStore } from './plugin.js'
export { useUiStore } from './ui.js'
