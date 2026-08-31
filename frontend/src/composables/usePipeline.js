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
  // Production injects App's bridge so prompt-hook generation/cancel state is
  // shared with useWriting. Tests and isolated callers retain a safe fallback.
  const pluginBridge = handlers.pluginBridge || usePluginBridge()
  const { broadcastPluginPipelineEvent, handlePromptHookRequest } = pluginBridge

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
      case 'writer_started':
        writing.pipeline.stateLabel = '执笔者续写'
        writing.pipeline.editor = {
          role: 'writer',
          status: 'running',
          detail: '读取回合案卷 · 直接成文',
          output: '',
        }
        break
      case 'writer_progress':
        // DraftReady 已携带权威全文；忽略异步通道中迟到的尾部 token，避免正文重复或退回 running。
        if (writing.pipeline.editor.status === 'done') break
        if (writing.pipeline.editor.status !== 'running') {
          writing.pipeline.editor = { role: 'writer', status: 'running', detail: '执笔中', output: '' }
        }
        writing.pipeline.editor.output += event.data.delta || ''
        if (scrollToBottom) scrollToBottom()
        break
      case 'editor_started':
        writing.pipeline.stateLabel = '编剧合并'
        writing.pipeline.editor = {
          role: 'editor',
          status: 'running',
          detail: '合并 · 润色 · 成文',
          output: '',
        }
        // Editor 逐字流式由 StreamingMessage 读 pipeline.editor.output 渲染，不再插占位消息
        break
      case 'editor_progress':
        // 累积编剧流式输出到 pipeline(StreamingMessage 实时渲染)
        if (writing.pipeline.editor.status === 'done') break
        if (writing.pipeline.editor.status !== 'running') {
          writing.pipeline.editor = { role: 'editor', status: 'running', detail: '生成中', output: '' }
        }
        writing.pipeline.editor.output += event.data.delta || ''
        if (scrollToBottom) scrollToBottom()
        break
      case 'draft_ready':
        writing.pipeline.editor = {
          ...writing.pipeline.editor,
          role: writing.pipeline.editor.role || 'editor',
          status: 'done',
          detail: writing.pipeline.editor.role === 'writer' ? '正文完成' : '成文完成',
          // 后端 DraftReady.text 是最终正文，优先于可能尚未排空的流式 delta。
          output: event.data?.text || writing.pipeline.editor.output || '',
        }
        writing.pipeline.stateLabel = '已产出'
        // 成文由 applyConversation 推入正式消息;StreamingMessage 随 showPipeline=false 消失
        break
      case 'quality_checked': {
        // B3：warn-only 质量门禁结果，不阻断 accept/postprocess
        const passed = !!event.data?.passed
        const warningCount = event.data?.warning_count || 0
        const errorCount = event.data?.error_count || 0
        const warnings = Array.isArray(event.data?.warnings) ? event.data.warnings : []
        writing.pipeline.quality = {
          passed,
          warningCount,
          errorCount,
          warnings,
          status: passed ? 'ok' : errorCount > 0 ? 'error' : 'warn',
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
        writing.pipeline.summary = event.data?.summarizer_enabled === false
          ? { status: 'idle', detail: '', charCount: 0 }
          : { status: 'running', detail: '提炼本轮剧情', charCount: 0 }
        writing.pipeline.postprocess = event.data?.postprocessor_enabled === false
          ? { status: 'idle', detail: '', knowledge: 0, variable: 0, task: 0, reason: '' }
          : { status: 'running', detail: '更新知识 · 变量 · 任务', knowledge: 0, variable: 0, task: 0, reason: '' }
        break
      case 'postprocess_done':
        if (writing.pipeline.summary.status === 'running') {
          writing.pipeline.summary = { status: 'error', detail: '摘要未产出', charCount: 0 }
        }
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
        if (writing.pipeline.summary.status === 'running') {
          writing.pipeline.summary = { status: 'error', detail: '摘要未产出', charCount: 0 }
        }
        writing.pipeline.postprocess = { status: 'error', detail: '后处理失败', knowledge: 0, variable: 0, task: 0, reason: event.data.reason || '' }
        break
      case 'postprocess_skipped':
        if (writing.pipeline.summary.status === 'running') {
          writing.pipeline.summary = { status: 'error', detail: '摘要未产出', charCount: 0 }
        }
        // 配置关闭不是一次模型调用，保持 idle，避免回顾中虚增“状态记账”阶段。
        writing.pipeline.postprocess = { status: 'idle', detail: '', knowledge: 0, variable: 0, task: 0, reason: event.data.reason || '' }
        break
      case 'summary_done':
        writing.pipeline.summary = {
          status: 'done',
          detail: `摘要 ${event.data.char_count || 0} 字`,
          charCount: event.data.char_count || 0,
        }
        break
      case 'error':
        // 事件流报错但 invoke 可能仍正常 resolve：置 error 态供成功路径甄别，
        // 否则「已完成」会覆盖错误标签（WritingScreen 失败提示条读 state==='error'）。
        writing.pipeline.state = 'error'
        writing.pipeline.stateLabel = `错误: ${event.data.message}`
        break
      case 'state_changed':
        // 状态变化已在其他事件中处理
        break
    }
  }

  return { handlePipelineEvent }
}
