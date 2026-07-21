/**
 * useWritingScreenAdapter — design/writing → store/composable 接线层。
 *
 * 职责：
 *   - 读 Pinia store 字段 → 组装 WritingScreen props
 *   - 将 8 个变体事件 + 写作意图事件转发到既有 handler
 *   - contentComponent（RichContent）由调用方注入，避免 adapter 顶层 import .vue
 *     （node:test 无法解析 SFC）
 *   - 删除前 tauri ask 确认
 *   - 派生质量门禁提示与子 Agent 重 roll 菜单
 *
 * 约束：不改 store 字段 / composable 签名；不碰 plugin-bridge / tauri-api 协议。
 */
import { computed, markRaw } from 'vue'
import { useWritingStore, useCampaignStore, useUiStore } from '../stores/index.js'
import { subagentRolesFromProvenance } from '../utils/pipelineTrace.js'

/**
 * @param {object} handlers
 * @param {object} [handlers.contentComponent] RichContent 等正文渲染组件（生产必传）
 */
export function useWritingScreenAdapter(handlers = {}) {
  const writing = useWritingStore()
  const campaign = useCampaignStore()
  const ui = useUiStore()

  const contentComponent = handlers.contentComponent
    ? markRaw(handlers.contentComponent)
    : null

  const title = computed(() => {
    if (writing.writingMode === 'campaign') {
      return campaign.activeCampaign?.name || 'Campaign 写作'
    }
    if (writing.writingMode === 'legacy') {
      return campaign.activeChar?.name || '写作'
    }
    return ui.pageTitle || 'StoryForge'
  })

  const canBranch = computed(
    () =>
      writing.writingMode === 'campaign' &&
      !!campaign.activeCampaign &&
      !!campaign.currentConversationId,
  )

  const greetingOptions = computed(() =>
    writing.canChooseGreeting ? writing.greetingOptions : [],
  )

  const composerDisabled = computed(() => writing.writingMode === 'none')

  const composerPlaceholder = computed(() =>
    writing.writingMode === 'none' ? '请先导入角色卡或打开 Campaign…' : '',
  )

  const qualityAcceptHint = computed(() => {
    const q = writing.pipeline.quality
    if (!q || q.passed) return null
    const errors = q.errorCount || 0
    const n = q.warningCount || (Array.isArray(q.warnings) ? q.warnings.length : 0)
    if (errors > 0) return `质量 Error ${errors}（采纳将确认）`
    if (!n) return null
    return `质量警告 ${n}`
  })

  const subagentRolesByMessage = computed(() => {
    const map = {}
    for (const m of writing.messages || []) {
      const v = m.variants?.[m.active_variant]
      map[m.id] = subagentRolesFromProvenance(v?.provenance) || []
    }
    return map
  })

  const screenProps = computed(() => ({
    title: title.value,
    durationText: '',
    messages: writing.messages,
    isWriting: writing.isWriting,
    pipeline: writing.pipeline,
    showPipeline: writing.showPipeline,
    streamingRoleLabel: writing.streamingRoleLabel,
    canBranch: canBranch.value,
    greetingOptions: greetingOptions.value,
    selectedGreetingIndex: writing.selectedGreetingIndex,
    composerDisabled: composerDisabled.value,
    composerPlaceholder: composerPlaceholder.value,
    qualityAcceptHint: qualityAcceptHint.value,
    contentComponent,
    subagentRolesByMessage: subagentRolesByMessage.value,
  }))

  async function onDeleteVariant(payload) {
    try {
      const { ask } = await import('@tauri-apps/plugin-dialog')
      const ok = await ask('确定删除这条消息？删除后该消息从对话移除。', {
        title: '删除确认',
        kind: 'warning',
      })
      if (!ok) return
    } catch {
      // 非 Tauri / 无 dialog 插件时直接删除（测试与浏览器预览）
    }
    return handlers.handleDeleteVariant?.(payload)
  }

  const screenEvents = {
    'start-writing': (text) => handlers.startWriting?.(text),
    cancel: () => handlers.cancelWriting?.(),
    import: () => handlers.handleImport?.(),
    'new-campaign': () => handlers.openNewCampaign?.(),
    'view-history': () => handlers.viewHistory?.(),
    'select-greeting': (index) => handlers.selectGreeting?.(index),
    reroll: (p) => handlers.handleReroll?.(p),
    'reroll-user': (p) => handlers.handleRerollUser?.(p),
    'switch-variant': (p) => handlers.handleSwitchVariant?.(p),
    'edit-variant': (p) => handlers.handleEditVariant?.(p),
    'accept-variant': (p) => handlers.handleAcceptVariant?.(p),
    'delete-variant': (p) => onDeleteVariant(p),
    'add-variant': (p) => handlers.handleAddVariant?.(p),
    branch: (p) => handlers.handleBranch?.(p),
  }

  return {
    screenProps,
    screenEvents,
  }
}
