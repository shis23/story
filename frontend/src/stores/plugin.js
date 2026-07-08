import { defineStore } from 'pinia'
import { ref } from 'vue'

// 插件系统状态。
// 来源 App.vue:102-110 + 129(模块级 Map/序号)。
// 注意:hookPluginHostRefs(Map)和序号不是响应式,放普通变量。
// showSidebarPlugins(App.vue:104)是死状态(声明后未读),不迁移。
export const usePluginStore = defineStore('plugin', () => {
  // 响应式状态
  const sidebarPlugins = ref([]) // App.vue:102 — 侧栏插件
  const hookPlugins = ref([]) // App.vue:103 — prompt hook 插件
  const hookPluginSlots = ref({}) // App.vue:105 — 插件 UI slot 注册表
  const pluginPipelineEvents = ref([]) // App.vue:106 — 最近流水线事件 feed
  const promptHookAuditRecords = ref([]) // App.vue:107 — prompt hook 审计记录

  // 非响应式(模块级单例,放 store 实例上)
  // hookPluginHostRefs: Map<pluginId, PluginHost ref> — App.vue:129
  //   composable 通过 setHookPluginHostRef / getHookPluginHostRef 操作
  const hookPluginHostRefs = new Map()
  let pluginPipelineEventSeq = 0 // App.vue:108

  // 常量(App.vue:109-110)
  const MAX_PLUGIN_PIPELINE_EVENTS = 100
  const MAX_PROMPT_HOOK_AUDIT_RECORDS = 100

  // host ref 管理(被 usePluginBridge composable 调用)
  function setHookPluginHostRef(pluginId, host) {
    if (host) hookPluginHostRefs.set(pluginId, host)
    else hookPluginHostRefs.delete(pluginId)
  }
  function getHookPluginHostRef(pluginId) {
    return hookPluginHostRefs.get(pluginId)
  }
  function allHookPluginHostRefs() {
    return Array.from(hookPluginHostRefs.values())
  }

  // 序号生成器(流水线事件用)
  function nextPipelineEventSeq() {
    pluginPipelineEventSeq += 1
    return pluginPipelineEventSeq
  }

  return {
    // state
    sidebarPlugins,
    hookPlugins,
    hookPluginSlots,
    pluginPipelineEvents,
    promptHookAuditRecords,
    // 非响应式
    hookPluginHostRefs,
    MAX_PLUGIN_PIPELINE_EVENTS,
    MAX_PROMPT_HOOK_AUDIT_RECORDS,
    // host ref 管理
    setHookPluginHostRef,
    getHookPluginHostRef,
    allHookPluginHostRefs,
    // 序号
    nextPipelineEventSeq,
  }
})
