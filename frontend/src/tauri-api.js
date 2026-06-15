/**
 * Tauri IPC 桥 —— 前端调 Rust 后端的入口
 */

import { invoke } from '@tauri-apps/api/core'

const isTauri = () => window.__TAURI_INTERNALS__ !== undefined

/**
 * 导入角色卡（从文件字节）
 */
export async function importCharacter(data) {
  if (isTauri()) {
    return await invoke('import_character', { data: Array.from(data) })
  }
  // Dev mock
  return {
    id: 'mock-1',
    name: '艾莉娅',
    description: '出身南方雨林的精灵游侠',
    tags: ['fantasy', 'elf'],
    creator: 'StoryForge',
    spec_version: '3.0',
    world_info_count: 2,
    has_renderable_assets: false,
    imported_at: '2026-06-14 12:00:00',
  }
}

/**
 * 列出所有已导入的角色卡
 */
export async function listCharacters() {
  if (isTauri()) {
    return await invoke('list_characters')
  }
  return []
}

/**
 * 获取单个角色卡详情（含世界书条目）
 */
export async function getCharacter(id) {
  if (isTauri()) {
    return await invoke('get_character', { id })
  }
  return null
}

/**
 * 删除角色卡
 */
export async function deleteCharacter(id) {
  if (isTauri()) {
    return await invoke('delete_character', { id })
  }
}

/**
 * 导入预设
 */
export async function importPreset(data) {
  if (isTauri()) {
    return await invoke('import_preset', { data: Array.from(data) })
  }
  return '预设导入成功（mock）'
}

/**
 * 获取版本号
 */
export async function getVersion() {
  if (isTauri()) {
    return await invoke('get_version')
  }
  return '0.1.0-dev'
}

/**
 * 更新世界书条目路由
 * @param {string} characterId - 角色卡 ID
 * @param {number} entryIndex - 条目索引
 * @param {string} route - 'Constant' | 'Selective' | 'Both' | 'Disabled'
 */
export async function updateWorldInfoRoute(characterId, entryIndex, route) {
  if (isTauri()) {
    return await invoke('update_world_info_route', { characterId, entryIndex, route })
  }
}

/**
 * 更新世界书条目的 keys/content/constant
 */
export async function updateWorldInfoEntry(characterId, entryIndex, keys, content, constant) {
  if (isTauri()) {
    return await invoke('update_world_info_entry', { characterId, entryIndex, keys, content, constant })
  }
}

/**
 * 新增世界书条目，返回新索引
 */
export async function addWorldInfoEntry(characterId, keys, content, constant) {
  if (isTauri()) {
    return await invoke('add_world_info_entry', { characterId, keys, content, constant })
  }
  return 0
}

/**
 * 删除世界书条目
 */
export async function deleteWorldInfoEntry(characterId, entryIndex) {
  if (isTauri()) {
    return await invoke('delete_world_info_entry', { characterId, entryIndex })
  }
}

/**
 * 拉取服务商可用模型列表（GET /v1/models）
 * 失败返回空数组（前端走模板兜底）
 */
export async function listModels(baseUrl, apiKey) {
  if (isTauri()) {
    return await invoke('list_models', { baseUrl, apiKey })
  }
  return []
}

// ─── LLM 连接管理 ──────────────────────────────────────────────────────────

/** 列出内置连接模板 */
export async function listConnectionTemplates() {
  if (isTauri()) {
    return await invoke('list_connection_templates')
  }
  return []
}

/** 列出已配置的连接（不含 key） */
export async function listConnections() {
  if (isTauri()) {
    return await invoke('list_connections')
  }
  return []
}

/** 查询当前活跃连接 */
export async function getActiveConnection() {
  if (isTauri()) {
    return await invoke('get_active_connection')
  }
  return null
}

/**
 * 创建连接
 * @param {Object} req - { templateId?, name, baseUrl, protocol, model, apiKey, toolMode, temperature?, topP?, maxTokens? }
 * @returns {Promise<string>} 新连接 id
 */
export async function createConnection(req) {
  if (isTauri()) {
    return await invoke('create_connection', {
      req: {
        template_id: req.templateId || null,
        name: req.name,
        base_url: req.baseUrl,
        protocol: req.protocol,
        model: req.model,
        api_key: req.apiKey,
        tool_mode: req.toolMode,
        temperature: req.temperature ?? null,
        top_p: req.topP ?? null,
        max_tokens: req.maxTokens ?? null,
      },
    })
  }
  return 'mock-conn-id'
}

/** 删除连接 */
export async function deleteConnection(id) {
  if (isTauri()) {
    return await invoke('delete_connection', { id })
  }
}

/** 设置活跃连接 */
export async function setActiveConnection(id) {
  if (isTauri()) {
    return await invoke('set_active_connection', { id })
  }
}

/**
 * 测试连接连通性
 * @param {Object} req - { baseUrl, apiKey, model, toolMode }
 * @returns {Promise<{success: boolean, message: string, latencyMs?: number}>}
 */
export async function testConnection(req) {
  if (isTauri()) {
    return await invoke('test_connection', {
      req: {
        base_url: req.baseUrl,
        api_key: req.apiKey,
        model: req.model,
        tool_mode: req.toolMode,
      },
    })
  }
  return { success: true, message: '（mock）连通成功', latencyMs: 42 }
}

// ─── M1 写作命令 ──────────────────────────────────────────────────────────

/**
 * 启动写作流水线（通过 Channel 接收流式事件）
 *
 * @param {string} intent - 用户写作意图
 * @param {string|null} characterId - 关联角色卡 ID
 * @param {function} onEvent - 事件回调 (event: {event_type, data}) => void
 * @returns {Promise<{text: string, conversation_id: string, node_id: string}>} 写作结果（含对话/节点 ID 供重 roll）
 */
export async function startWriting(intent, characterId, onEvent) {
  if (isTauri()) {
    const { Channel } = await import('@tauri-apps/api/core')
    const channel = new Channel()
    channel.onmessage = (event) => {
      if (onEvent) onEvent(event)
    }
    return await invoke('start_writing', {
      intent,
      characterId: characterId || null,
      onEvent: channel,
    })
  }
  // Dev mock：模拟流水线
  const text = await mockStartWriting(intent, onEvent)
  return {
    text,
    conversation_id: 'mock-conv-1',
    node_id: 'mock-node-1',
  }
}

/**
 * 取消当前运行的写作流水线
 * @returns {Promise<boolean>} 是否有运行中的写作被取消
 */
export async function cancelWriting() {
  if (isTauri()) {
    return await invoke('cancel_writing')
  }
  return false
}

/**
 * 重 roll（整体/只重编剧/只重某子 Agent，可附 hint）
 *
 * @param {Object} req - { conversationId, nodeId, targets: [{kind}], hint?, seed? }
 *   kind: 'director' | 'editor' | 'subagent:<角色名>'；targets 为空 = 整体重 roll
 * @param {function} onEvent - 事件回调
 * @returns {Promise<string>} 新 variant 的成文
 */
export async function regenerate(req, onEvent) {
  if (isTauri()) {
    const { Channel } = await import('@tauri-apps/api/core')
    const channel = new Channel()
    channel.onmessage = (event) => {
      if (onEvent) onEvent(event)
    }
    return await invoke('regenerate', {
      req: {
        conversation_id: req.conversationId,
        node_id: req.nodeId,
        targets: req.targets || [],
        hint: req.hint || null,
        seed: req.seed ?? null,
      },
      onEvent: channel,
    })
  }
  // Dev mock
  return mockStartWriting(req?.hint || '重 roll', onEvent)
}

// ─── M1 对话命令 ──────────────────────────────────────────────────────────

/**
 * 列出所有对话
 */
export async function listConversations() {
  if (isTauri()) {
    return await invoke('list_conversations')
  }
  return []
}

/**
 * 获取对话详情
 */
export async function getConversation(id) {
  if (isTauri()) {
    return await invoke('get_conversation', { id })
  }
  return null
}

// ─── M1 对话操作命令 ──────────────────────────────────────────────────────

/** 编辑当前变体内容 */
export async function editVariant(conversationId, nodeId, newContent) {
  if (isTauri()) {
    return await invoke('edit_variant', { conversationId, nodeId, newContent })
  }
}

/** 采纳当前变体（Draft → Final） */
export async function acceptVariant(conversationId, nodeId) {
  if (isTauri()) {
    return await invoke('accept_variant', { conversationId, nodeId })
  }
}

/** 软删除当前变体（→ Discarded） */
export async function softDeleteVariant(conversationId, nodeId) {
  if (isTauri()) {
    return await invoke('soft_delete_variant', { conversationId, nodeId })
  }
}

/** 添加新变体（分支/swipe），返回新 variant 索引 */
export async function addVariant(conversationId, nodeId, content, provenance) {
  if (isTauri()) {
    return await invoke('add_variant', { conversationId, nodeId, content, provenance: provenance || null })
  }
  return 0
}

/** 切换变体（左右滑） */
export async function switchVariant(conversationId, nodeId, index) {
  if (isTauri()) {
    return await invoke('switch_variant', { conversationId, nodeId, index })
  }
}

// ─── M1 日志命令 ──────────────────────────────────────────────────────────

/**
 * 查询日志
 */
export async function logQuery(filter = {}) {
  if (isTauri()) {
    return await invoke('log_query', { filter })
  }
  return []
}

/**
 * 清空日志
 */
export async function logClear(kind = null) {
  if (isTauri()) {
    return await invoke('log_clear', { kind })
  }
}

/**
 * 导出日志 bundle
 */
export async function logExportBundle(redactContent = true) {
  if (isTauri()) {
    return await invoke('log_export_bundle', { redactContent })
  }
  return { exported_at: new Date().toISOString(), counts: { backend: 0, llm: 0, frontend: 0 } }
}

/**
 * 前端日志上报（console.log/warn/error 转发到后端）
 * @param {'debug'|'info'|'warn'|'error'} level
 * @param {string} message
 */
export async function logAppendFrontend(level, message) {
  if (isTauri()) {
    return await invoke('log_append_frontend', { level, message })
  }
}

// ─── M2 记忆系统命令 ──────────────────────────────────────────────────────

/** 配置嵌入 API */
export async function configureEmbedder(endpoint, apiKey, model, dim) {
  if (isTauri()) {
    return await invoke('configure_embedder', { endpoint, apiKey, model, dim })
  }
}

/** 获取当前嵌入配置（不含 key） */
export async function getEmbedConfig() {
  if (isTauri()) {
    return await invoke('get_embed_config')
  }
  return null
}

/** 手动触发对话归档 */
export async function archiveConversation(conversationId) {
  if (isTauri()) {
    return await invoke('archive_conversation', { conversationId })
  }
  return 0
}

// ─── Meta Agent 命令 ──────────────────────────────────────────────────────

/** 接受并执行 Patch */
export async function metaAcceptPatch(patchId) {
  if (isTauri()) {
    return await invoke('meta_accept_patch', { patchId })
  }
}

// ─── Dev mock（无 Tauri 时的模拟流水线）────────────────────────────────────

async function mockStartWriting(intent, onEvent) {
  const delay = (ms) => new Promise((r) => setTimeout(r, ms))

  onEvent?.({ event_type: 'started', data: { session_id: 'mock-session' } })
  await delay(300)

  onEvent?.({ event_type: 'director_started', data: {} })
  await delay(800)
  onEvent?.({
    event_type: 'director_done',
    data: { scene_brief: '一场雨中告别戏', subagent_count: 1 },
  })
  await delay(200)

  onEvent?.({
    event_type: 'subagent_started',
    data: { character_id: 'Seraphina', index: 0, total: 1 },
  })
  await delay(1500)
  onEvent?.({
    event_type: 'subagent_done',
    data: { character_id: 'Seraphina', index: 0 },
  })
  await delay(200)

  onEvent?.({ event_type: 'editor_started', data: {} })
  await delay(1200)
  onEvent?.({ event_type: 'draft_ready', data: { text: mockDraftText } })
  await delay(200)

  onEvent?.({ event_type: 'state_changed', data: { state: 'Committed' } })

  return mockDraftText
}

const mockDraftText = `## 雨中告别

雨幕低垂，将整个世界笼罩在一片朦胧的灰蓝色调中。屋檐下的积水映出两道身影。

Seraphina 站在那里，银色的长发被雨水浸透，水珠沿着发梢滴落。

「你知道的……」她的声音很轻，几乎要被雨声淹没。

「有些告别，是为了更好的重逢。」

*她伸出手指，轻轻触碰你的掌心。那一刻的温度，足以温暖此后所有漫长的雨季。*`
