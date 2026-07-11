// Pipeline composable — 迁移自 App.vue:1100-1211 的 handlePipelineEvent。
// 它是一个 reducer:接收流水线事件,更新 writingStore.pipeline 的各字段
// (state/stateLabel/director/subagents/editor/postprocess)。
//
// 消费:useWritingStore(pipeline)、useCampaignStore(instanceNameMap 读取)、
// usePluginStore via usePluginBridge(broadcastPluginPipelineEvent / handlePromptHookRequest)。
// 只 import 不修改:无 tauri-api 直接调用。

import { useWritingStore } from '../stores/writing.js'
import { useCampaignStore } from '../stores/campaign.js'
import { usePluginBridge } from './usePluginBridge.js'

export function usePipeline(handlers = {}) {
  const writing = useWritingStore()
  const campaign = useCampaignStore()
  // 广播(pipeline 事件喂给插件事件 feed)与 prompt hook 编排来自 usePluginBridge。
  const { broadcastPluginPipelineEvent, handlePromptHookRequest } = usePluginBridge()

  // 范围外依赖(由调用方注入):App.vue 里 scrollToBottom 是 DOM 自动滚动逻辑
  // (读 messagesContainer ref),不属于本 composable 迁移范围。
  const scrollToBottom = handlers.scrollToBottom

  // 处理流水线事件(更新 UI)。App.vue:1100-1211。
  function handlePipelineEvent(event) {
    broadcastPluginPipelineEvent(event)

    switch (event.event_type) {
      case 'director_started':
        writing.pipeline.stateLabel = '导演规划中'
        writing.pipeline.director = { status: 'running', detail: '解析意图 · 检索世界书', output: '' }
        break
      case 'director_progress':
        // 累积导演流式输出
        if (writing.pipeline.director.status !== 'running') {
          writing.pipeline.director = { status: 'running', detail: '导演思考中', output: '' }
        }
        writing.pipeline.director.output += event.data.delta || ''
        break
      case 'director_done':
        writing.pipeline.director = {
          status: 'done',
          detail: `规划完成 · 分配 ${event.data.subagent_count} 个角色`,
          output: writing.pipeline.director.output, // 保留已累积的输出
        }
        // 预填充 subagents 数组(避免稀疏数组导致 Vue 响应式失效，M-15)
        writing.pipeline.subagents = Array.from({ length: event.data.subagent_count }, () => ({
          id: '', name: '', emoji: '🎭', status: 'pending', progress: 0,
        }))
        writing.pipeline.stateLabel = '子 Agent 并行表演'
        break
      case 'subagent_started':
        writing.pipeline.subagents[event.data.index] = {
          id: event.data.character_id,
          name: campaign.instanceNameMap[event.data.character_id] || event.data.character_id,
          emoji: event.data.emoji || '🎭',
          status: 'running',
          progress: 0,
          output: '',
          _expanded: false,
        }
        break
      case 'subagent_progress':
        if (writing.pipeline.subagents[event.data.index]) {
          writing.pipeline.subagents[event.data.index].progress = Math.min(
            100,
            (writing.pipeline.subagents[event.data.index].progress || 0) + 15,
          )
        }
        break
      case 'subagent_done':
        if (writing.pipeline.subagents[event.data.index]) {
          writing.pipeline.subagents[event.data.index].status = 'done'
          writing.pipeline.subagents[event.data.index].progress = 100
          writing.pipeline.subagents[event.data.index].output = event.data.full_text || ''
        }
        break
      case 'subagent_cancelled':
        if (writing.pipeline.subagents[event.data.index]) {
          writing.pipeline.subagents[event.data.index].status = 'cancelled'
        }
        break
      case 'editor_started':
        writing.pipeline.stateLabel = '编剧合并'
        writing.pipeline.editor = { status: 'running', detail: '合并 · 润色 · 成文', output: '' }
        // Editor 逐字流式由 StreamingMessage 读 pipeline.editor.output 渲染，不再插占位消息
        break
      case 'editor_progress':
        // 累积编剧流式输出到 pipeline(StreamingMessage 实时渲染)
        if (writing.pipeline.editor.status !== 'running') {
          writing.pipeline.editor = { status: 'running', detail: '生成中', output: '' }
        }
        writing.pipeline.editor.output += event.data.delta || ''
        if (scrollToBottom) scrollToBottom()
        break
      case 'draft_ready':
        writing.pipeline.editor = { status: 'done', detail: '成文完成' }
        writing.pipeline.stateLabel = '已产出'
        // 成文由 applyConversation 推入正式消息;StreamingMessage 随 showPipeline=false 消失
        break
      case 'quality_checked': {
        // B3：warn-only 质量门禁结果，不阻断 accept/postprocess
        const passed = !!event.data?.passed
        const warningCount = event.data?.warning_count || 0
        const warnings = Array.isArray(event.data?.warnings) ? event.data.warnings : []
        writing.pipeline.quality = {
          passed,
          warningCount,
          warnings,
          status: passed ? 'ok' : 'warn',
        }
        if (!passed && warningCount > 0) {
          writing.pipeline.stateLabel = `已产出 · 质量警告 ${warningCount}`
          if (writing.pipeline.editor?.status === 'done') {
            writing.pipeline.editor = {
              ...writing.pipeline.editor,
              detail: `成文完成 · 质量警告 ${warningCount}`,
            }
          }
        }
        break
      }
      case 'prompt_hook_request':
        handlePromptHookRequest(event.data)
        break
      case 'postprocess_started':
        writing.pipeline.postprocess = { status: 'running', detail: '提取知识 · 更新变量 · 检测任务', knowledge: 0, variable: 0, task: 0, reason: '' }
        break
      case 'postprocess_done':
        writing.pipeline.postprocess = {
          status: 'done',
          detail: `知识 ${event.data.knowledge_count || 0} · 变量 ${event.data.variable_count || 0} · 任务 ${event.data.task_count || 0}`,
          knowledge: event.data.knowledge_count || 0,
          variable: event.data.variable_count || 0,
          task: event.data.task_count || 0,
          reason: '',
        }
        break
      case 'postprocess_failed':
        writing.pipeline.postprocess = { status: 'error', detail: '后处理失败', knowledge: 0, variable: 0, task: 0, reason: event.data.reason || '' }
        break
      case 'postprocess_skipped':
        writing.pipeline.postprocess = { status: 'done', detail: '已跳过', knowledge: 0, variable: 0, task: 0, reason: event.data.reason || '' }
        break
      case 'summary_done':
        // 摘要是后处理子步骤，记到 postprocess detail
        if (writing.pipeline.postprocess.status === 'running') {
          writing.pipeline.postprocess.detail = `摘要 ${event.data.char_count || 0} 字 · 提取中`
        }
        break
      case 'error':
        writing.pipeline.stateLabel = `错误: ${event.data.message}`
        break
      case 'state_changed':
        // 状态变化已在其他事件中处理
        break
    }
  }

  return { handlePipelineEvent }
}
