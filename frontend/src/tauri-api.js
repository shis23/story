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

/** 列出所有预设（摘要） */
export async function listPresets() {
  if (isTauri()) {
    return await invoke('list_presets')
  }
  return []
}

/** 获取预设详情（含每条 prompt 和正则） */
export async function getPreset(id) {
  if (isTauri()) {
    return await invoke('get_preset', { id })
  }
  return null
}

/** 获取当前活跃预设 */
export async function getActivePreset() {
  if (isTauri()) {
    return await invoke('get_active_preset')
  }
  return null
}

/** 设置/清除当前活跃预设 */
export async function setActivePreset(id) {
  if (isTauri()) {
    return await invoke('set_active_preset', { id: id || null })
  }
}

/** 删除预设 */
export async function deletePreset(id) {
  if (isTauri()) {
    return await invoke('delete_preset', { id })
  }
}

/** 更新预设中某条 prompt 的内容和/或启用状态 */
export async function updatePresetPrompt(presetId, promptIndex, content, enabled) {
  if (isTauri()) {
    return await invoke('update_preset_prompt', { presetId, promptIndex, content, enabled })
  }
}

/** 更新预设中某条 regex 的禁用状态 */
export async function updatePresetRegex(presetId, regexIndex, disabled) {
  if (isTauri()) {
    return await invoke('update_preset_regex', { presetId, regexIndex, disabled })
  }
}

/** 列出全局正则脚本 */
export async function listGlobalRegexScripts() {
  if (isTauri()) {
    return await invoke('list_global_regex_scripts')
  }
  return []
}

/** 从 ST settings JSON 导入全局正则脚本 */
export async function importGlobalRegexSettings(settingsJson) {
  if (isTauri()) {
    return await invoke('import_global_regex_settings', { settingsJson })
  }
  return 0
}

/** 清空全局正则脚本 */
export async function clearGlobalRegexScripts() {
  if (isTauri()) {
    return await invoke('clear_global_regex_scripts')
  }
}

/** 更新全局正则禁用状态 */
export async function updateGlobalRegex(regexIndex, disabled) {
  if (isTauri()) {
    return await invoke('update_global_regex', { regexIndex, disabled })
  }
}

/** 将 ST 预设的 prompts 转换为模块（返回转换数量） */
export async function importPresetAsModules(presetId) {
  if (isTauri()) {
    return await invoke('import_preset_as_modules', { presetId })
  }
}

// ─── M4 插件命令 ──────────────────────────────────────────────────────────

/** 列出所有已安装插件 */
export async function listPlugins() {
  if (isTauri()) {
    return await invoke('list_plugins')
  }
}

/** 安装插件（传入 manifest JSON 字符串） */
export async function installPlugin(manifestJson) {
  if (isTauri()) {
    return await invoke('install_plugin', { manifestJson })
  }
}

/** 卸载插件 */
export async function uninstallPlugin(id) {
  if (isTauri()) {
    return await invoke('uninstall_plugin', { id })
  }
}

/** 启用/禁用插件 */
export async function setPluginEnabled(id, enabled) {
  if (isTauri()) {
    return await invoke('set_plugin_enabled', { id, enabled })
  }
}

// ─── 预设/模块系统 ──────────────────────────────────────────────────────────

/** 列出所有模块（内置+自定义，含 enabled 状态） */
export async function listModules() {
  if (isTauri()) {
    return await invoke('list_modules')
  }
}

/** 更新模块内容或启停状态 */
export async function updateModule(id, content, enabled) {
  if (isTauri()) {
    return await invoke('update_module', { id, content, enabled })
  }
}

/** 列出所有 Profile */
export async function listProfiles() {
  if (isTauri()) {
    return await invoke('list_profiles')
  }
}

/** 获取当前活跃 Profile */
export async function getActiveProfile() {
  if (isTauri()) {
    return await invoke('get_active_profile')
  }
}

/** 保存/更新 Profile */
export async function saveProfile(profileJson) {
  if (isTauri()) {
    return await invoke('save_profile', { profileJson })
  }
}

/** 设置活跃 Profile */
export async function setActiveProfile(id) {
  if (isTauri()) {
    return await invoke('set_active_profile', { id })
  }
}

// ─── Agent Profile Config 命令 ─────────────────────────────────────────────

/** 列出所有 Agent Profile 配置（摘要） */
export async function listAgentProfileConfigs() {
  if (isTauri()) {
    return await invoke('list_agent_profile_configs')
  }
  return []
}

/** 获取指定 Agent Profile 配置（完整） */
export async function getAgentProfileConfig(id) {
  if (isTauri()) {
    return await invoke('get_agent_profile_config', { id })
  }
  return null
}

/** 获取当前活跃 Agent Profile 配置 */
export async function getActiveAgentProfileConfig() {
  if (isTauri()) {
    return await invoke('get_active_agent_profile_config')
  }
  return null
}

/** 保存/更新 Agent Profile 配置 */
export async function saveAgentProfileConfig(configJson) {
  if (isTauri()) {
    return await invoke('save_agent_profile_config', { configJson })
  }
}

/** 导出 Agent Profile 配置为 JSON 字符串 */
export async function exportAgentProfileConfig(id) {
  if (isTauri()) {
    return await invoke('export_agent_profile_config', { id })
  }
  return ''
}

/** 从 JSON 字符串导入 Agent Profile 配置 */
export async function importAgentProfileConfig(configJson) {
  if (isTauri()) {
    return await invoke('import_agent_profile_config', { configJson })
  }
  return null
}

/** 删除 Agent Profile 配置（内置默认不可删除） */
export async function deleteAgentProfileConfig(id) {
  if (isTauri()) {
    return await invoke('delete_agent_profile_config', { id })
  }
}

/** 设置活跃 Agent Profile 配置 */
export async function setActiveAgentProfileConfig(id) {
  if (isTauri()) {
    return await invoke('set_active_agent_profile_config', { id })
  }
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
 * 更新世界书条目的 keys/content/constant/is_global/depth/order
 */
export async function updateWorldInfoEntry(characterId, entryIndex, keys, content, constant, isGlobal = false, depth = 2, order = 100) {
  if (isTauri()) {
    return await invoke('update_world_info_entry', { characterId, entryIndex, keys, content, constant, isGlobal, depth, order })
  }
}

/**
 * 新增世界书条目，返回新索引
 */
export async function addWorldInfoEntry(characterId, keys, content, constant, isGlobal = false) {
  if (isTauri()) {
    return await invoke('add_world_info_entry', { characterId, keys, content, constant, isGlobal })
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
   * @param {Object} req - { templateId?, name, baseUrl, protocol, model, apiKey, toolMode, temperature?, topP?, maxTokens?, reasoning?, extra? }
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
        max_tokens_explicit: req.maxTokensExplicit ?? false,
        reasoning: req.reasoning ?? 'disabled',
        // P3-3：厂商扩展参数（thinking/reasoning_effort 等），透传到请求体顶层
        extra: req.extra ?? null,
      },
    })
  }
  return 'mock-conn-id'
}

/**
 * 获取单条连接详情供编辑（不含 api_key 明文）
 * @returns {Promise<null | {
 *   id: string, name: string, base_url: string, model: string,
 *   protocol: string, tool_mode: string, temperature?: number, top_p?: number,
 *   max_tokens?: number, max_tokens_explicit: boolean, reasoning: string,
 *   extra?: object, has_api_key: boolean, active: boolean
 * }>}
 */
export async function getConnection(id) {
  if (isTauri()) {
    return await invoke('get_connection', { id })
  }
  return null
}

/**
 * 更新已有连接。apiKey 空串 = 保留原密钥。
 * @param {Object} req - { id, name, baseUrl, protocol, model, apiKey?, toolMode, temperature?, topP?, maxTokens?, maxTokensExplicit?, reasoning?, extra? }
 */
export async function updateConnection(req) {
  if (isTauri()) {
    return await invoke('update_connection', {
      req: {
        id: req.id,
        name: req.name,
        base_url: req.baseUrl,
        protocol: req.protocol,
        model: req.model,
        api_key: req.apiKey ?? '',
        tool_mode: req.toolMode,
        temperature: req.temperature ?? null,
        top_p: req.topP ?? null,
        max_tokens: req.maxTokens ?? null,
        max_tokens_explicit: req.maxTokensExplicit ?? false,
        reasoning: req.reasoning ?? 'disabled',
        extra: req.extra ?? null,
      },
    })
  }
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
 * @param {Object} req - { baseUrl, apiKey, model, protocol, toolMode }
 * @returns {Promise<{success: boolean, message: string, latencyMs?: number}>}
 */
export async function testConnection(req) {
  if (isTauri()) {
    return await invoke('test_connection', {
      req: {
        base_url: req.baseUrl,
        api_key: req.apiKey,
        model: req.model,
        protocol: req.protocol,
        tool_mode: req.toolMode,
        reasoning: req.reasoning ?? 'disabled',
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
 * @param {string|null} conversationId - 已有对话 ID（追加到已有对话；null = 新建）
 * @param {string|null} openingMessage - 新建 legacy 对话时使用的角色卡开场白
 * @returns {Promise<{text: string, conversation_id: string, node_id: string}>} 写作结果（含对话/节点 ID 供重 roll）
 */
export async function startWriting(intent, characterId, onEvent, conversationId, openingMessage, generationMode) {
  if (isTauri()) {
    const { Channel } = await import('@tauri-apps/api/core')
    const channel = new Channel()
    channel.onmessage = (event) => {
      if (onEvent) onEvent(event)
    }
    return await invoke('start_writing', {
      intent,
      characterId: characterId || null,
      conversationId: conversationId || null,
      openingMessage: openingMessage || null,
      generationMode: generationMode || null,
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
export async function pluginPromptHookResult(requestId, messages, error = null) {
  if (isTauri()) {
    return await invoke('plugin_prompt_hook_result', {
      requestId,
      messages: Array.isArray(messages) ? messages : null,
      error: error || null,
    })
  }
}

export async function cancelWriting() {
  if (isTauri()) {
    return await invoke('cancel_writing')
  }
  return false
}

/**
 * 重 roll（整体/只重编剧/只重某子 Agent，可附 hint）
 *
 * @param {Object} req - { conversationId, nodeId, targets: [{kind}], generationMode?, hint?, seed? }
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
        generation_mode: req.generationMode || null,
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

export async function deleteConversation(conversationId) {
  if (isTauri()) {
    return await invoke('delete_conversation', { conversationId })
  }
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

/** 采纳当前变体（Draft → Final）
 * @param {boolean} [forceAccept=false] Quality Error 时需二次确认后传 true → Degraded
 */
export async function acceptVariant(
  conversationId,
  nodeId,
  forceAccept = false,
  selectedMutationIndices = null,
) {
  if (isTauri()) {
    return await invoke('accept_variant', {
      conversationId,
      nodeId,
      forceAccept: !!forceAccept,
      selectedMutationIndices,
    })
  }
}

/** 软删除当前变体（→ Discarded） */
export async function softDeleteVariant(conversationId, nodeId) {
  if (isTauri()) {
    return await invoke('soft_delete_variant', { conversationId, nodeId })
  }
}

/** 删除指定消息及其后所有消息（截断对话 = 撤销从这条开始的写作） */
export async function deleteMessageFrom(conversationId, nodeId) {
  if (isTauri()) {
    return await invoke('delete_message_from', { conversationId, nodeId })
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
 * A2：获取单条 LLM 调用详情（含 token 统计、缓存命中）
 */
export async function logGetLlmCall(id) {
  if (isTauri()) {
    return await invoke('log_get_llm_call', { id })
  }
  return null
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

// ─── P1 角色识别 / CharacterCard / Campaign ────────────────────────────────

/**
 * 跑角色识别 Agent，为已导入的扁平 Character 建 CharacterCard
 * 失败时降级建单角色 Protagonist
 * @param {string} sourceCharacterId - 原角色卡 ID
 * @param {{force?: boolean}} options
 * @returns {Promise<{id, name, source_character_id, definition_count, character_count, extracted, extraction_status, extraction_message}>}
 */
export async function extractCharacters(sourceCharacterId, options = {}) {
  if (isTauri()) {
    return await invoke('extract_characters', {
      sourceCharacterId,
      force: !!options.force,
    })
  }
  return {
    id: 'mock-card-1',
    name: 'Mock Card',
    source_character_id: sourceCharacterId,
    definition_count: 1,
    character_count: 1,
    extracted: false,
    extraction_status: options.force ? 'fallback' : 'unknown',
    extraction_message: options.force ? '识别失败，已按单角色处理，可重新识别。' : null,
  }
}

/** 列出所有 CharacterCard */
export async function listCards() {
  if (isTauri()) {
    return await invoke('list_cards')
  }
  return []
}

// ─── Card Studio Phase 1 ────────────────────────────────────────────────

export async function cardstudioListProjects() {
  if (isTauri()) {
    return await invoke('cardstudio_list_projects')
  }
  return []
}

export async function cardstudioCreateProject(name, brief) {
  if (isTauri()) {
    return await invoke('cardstudio_create_project', { name, brief })
  }
  return {
    id: 'mock-cardstudio-1',
    name,
    brief,
    mode: 'from_scratch',
    current_stage: 'brief',
    stage_status: { brief: 'ready' },
    artifacts: {
      name,
      description: '',
      personality: '',
      scenario: '',
      first_mes: '',
      tags: [],
      creator: '',
      worldview_entries: [],
      notes: brief,
    },
    last_error: null,
    last_stage_output: null,
    imported_character_id: null,
    source_character_id: null,
    source_stored_id: null,
    created_at: '2026-07-22 00:00:00',
    updated_at: '2026-07-22 00:00:00',
  }
}

/** C: 从已有角色卡创建修订项目（默认另存） */
export async function cardstudioCreateFromCharacter(characterId, brief = null) {
  if (isTauri()) {
    return await invoke('cardstudio_create_from_character', {
      characterId,
      brief: brief || null,
    })
  }
  return {
    id: 'mock-revise-1',
    name: '修订（mock）',
    mode: 'from_existing_card',
    current_stage: 'review',
    source_character_id: characterId,
  }
}

/** B: 从小说正文创建改编项目（随后可 prefill） */
export async function cardstudioCreateFromNovel(name, brief, novelTitle, novelText) {
  if (isTauri()) {
    return await invoke('cardstudio_create_from_novel', {
      name: name || '',
      brief: brief || '',
      novelTitle: novelTitle || '',
      novelText: novelText || '',
    })
  }
  return {
    id: 'mock-novel-1',
    name: name || '小说改编（mock）',
    mode: 'from_novel',
    current_stage: 'basic',
    novel_title: novelTitle,
  }
}

/** B: 对 FromNovel 项目做 LLM 预填 */
export async function cardstudioPrefillFromNovel(id, userNote = null, includeStyle = true) {
  if (isTauri()) {
    return await invoke('cardstudio_prefill_from_novel', {
      id,
      userNote: userNote || null,
      includeStyle,
    })
  }
  return null
}

export async function cardstudioGetProject(id) {
  if (isTauri()) {
    return await invoke('cardstudio_get_project', { id })
  }
  return null
}

export async function cardstudioDeleteProject(id) {
  if (isTauri()) {
    return await invoke('cardstudio_delete_project', { id })
  }
  return true
}

export async function cardstudioUpdateArtifacts(id, artifacts) {
  if (isTauri()) {
    return await invoke('cardstudio_update_artifacts', { id, artifacts })
  }
  return null
}

export async function cardstudioSetStage(id, stageId) {
  if (isTauri()) {
    return await invoke('cardstudio_set_stage', { id, stageId })
  }
  return null
}

export async function cardstudioSetOptions(id, { allowAiFreewrite = null, stagePackId = null } = {}) {
  if (isTauri()) {
    return await invoke('cardstudio_set_options', {
      id,
      allowAiFreewrite,
      stagePackId,
    })
  }
  return null
}

export async function cardstudioRunChecks(id) {
  if (isTauri()) {
    return await invoke('cardstudio_run_checks', { id })
  }
  return { ok: false, issues: [], score: 0, summary: '', source: 'rule' }
}

export async function cardstudioRunReview(id, userNote = null, useLlm = true) {
  if (isTauri()) {
    return await invoke('cardstudio_run_review', {
      id,
      userNote: userNote || null,
      useLlm,
    })
  }
  return { ok: false, issues: [], score: 0, summary: 'mock', source: 'rule' }
}

export async function cardstudioCompile(id) {
  if (isTauri()) {
    return await invoke('cardstudio_compile', { id })
  }
  return { st_card_json: {}, warnings: [], character_name: '' }
}

/** 出卡质量闸门：compile → JSON/PNG round-trip 真实导入 → 确定性检查报告 */
export async function cardstudioExportGate(id) {
  if (isTauri()) {
    return await invoke('cardstudio_export_gate', { id })
  }
  return { pass: true, character_name: '', warnings: [], json_checks: [], png_checks: [] }
}

/** 导出编译产物为 ST PNG 卡（返回字节数组） */
export async function cardstudioExportPng(id) {
  if (isTauri()) {
    return await invoke('cardstudio_export_png', { id })
  }
  return []
}

export async function cardstudioCompleteManualStage(id, stageId) {
  if (isTauri()) {
    return await invoke('cardstudio_complete_manual_stage', { id, stageId })
  }
  return null
}

export async function cardstudioRunStage(id, stageId, userNote = null) {
  if (isTauri()) {
    return await invoke('cardstudio_run_stage', {
      id,
      stageId,
      userNote: userNote || null,
    })
  }
  return null
}

export async function cardstudioImportCompiled(id) {
  if (isTauri()) {
    return await invoke('cardstudio_import_compiled', { id })
  }
  return {
    character: {
      id: 'mock-imported',
      name: 'mock',
      description: '',
      tags: [],
      creator: '',
      spec_version: '3.0',
      world_info_count: 0,
      has_renderable_assets: false,
      imported_at: '2026-07-22 00:00:00',
    },
    card_id: 'mock-card',
    source_character_id: 'mock-imported-domain',
    warnings: [],
  }
}

export async function cardstudioListStages() {
  if (isTauri()) {
    return await invoke('cardstudio_list_stages')
  }
  return ['brief', 'basic', 'personality', 'worldview', 'opening', 'review', 'compile_import']
}

/** 获取 CharacterCard 详情（含 character_definitions） */
export async function getCard(id) {
  if (isTauri()) {
    return await invoke('get_card', { id })
  }
  return null
}

export async function deleteCard(id) {
  if (isTauri()) {
    return await invoke('delete_card', { id })
  }
}

/**
 * 开档：建 Campaign，实例化所有 Protagonist/Supporting 角色
 * @param {string} cardId - CharacterCard ID
 * @param {string} name - 档名
 * @param {string|null} openingMessage - 新建 Campaign 对话时使用的角色卡开场白
 */
export async function createCampaign(cardId, name, openingMessage = null) {
  if (isTauri()) {
    return await invoke('create_campaign', { cardId, name, openingMessage: openingMessage || null })
  }
  return { id: 'mock-campaign-1', card_id: cardId, name, instance_count: 1 }
}

/** 开场壳选择落库：改写 Campaign 绑定会话的首条开场白（仅开场态可用） */
export async function applyCampaignOpening(campaignId, content) {
  if (isTauri()) {
    return await invoke('apply_campaign_opening', { campaignId, content })
  }
}

/** 列出 Campaign（可按 card_id 过滤） */
export async function forkCampaign(sourceCampaignId, forkNodeId, name) {
  if (isTauri()) {
    return await invoke('fork_campaign', { sourceCampaignId, forkNodeId, name })
  }
  return {
    id: `mock-fork-${Date.now()}`,
    card_id: 'mock-card-1',
    name,
    fork_from: [sourceCampaignId, forkNodeId],
    instance_count: 1,
    conversation_id: 'mock-conversation-1',
  }
}

export async function listCampaigns(cardId = null) {
  if (isTauri()) {
    return await invoke('list_campaigns', { cardId })
  }
  return []
}

/** 获取单个 Campaign 详情 */
export async function getCampaign(id) {
  if (isTauri()) {
    return await invoke('get_campaign', { id })
  }
  return null
}

/**
 * 删除整局活动（一 Campaign 一对话：级联会话 + 实例/知识/任务/总结）。
 * @param {string} id - Campaign id
 */
export async function deleteCampaign(id) {
  if (isTauri()) {
    return await invoke('delete_campaign', { id })
  }
}

/** 设置活跃 Campaign */
export async function setActiveCampaign(id) {
  if (isTauri()) {
    return await invoke('set_active_campaign', { id })
  }
}

// ─── Campaign 本局世界书（卡只读 / 活动可写）──────────────────────────────

/** 列出本局世界书（惰性从卡模板拷贝） */
export async function listCampaignWorldInfo(campaignId) {
  if (isTauri()) {
    return await invoke('list_campaign_world_info', { campaignId })
  }
  return { campaign_id: campaignId, entry_count: 0, constant_count: 0, selective_count: 0, entries: [] }
}

/** 新增本局世界书条目 */
export async function addCampaignWorldInfoEntry(req) {
  if (isTauri()) {
    return await invoke('add_campaign_world_info_entry', {
      req: {
        campaign_id: req.campaignId,
        keys: req.keys || [],
        content: req.content || '',
        constant: !!req.constant,
        depth: req.depth ?? 2,
        order: req.order ?? 100,
      },
    })
  }
  return 0
}

/** 更新本局世界书条目 */
export async function updateCampaignWorldInfoEntry(req) {
  if (isTauri()) {
    return await invoke('update_campaign_world_info_entry', {
      req: {
        campaign_id: req.campaignId,
        entry_index: req.entryIndex,
        keys: req.keys || [],
        content: req.content || '',
        constant: !!req.constant,
        disabled: !!req.disabled,
        depth: req.depth ?? 2,
        order: req.order ?? 100,
        route: req.route || 'Selective',
      },
    })
  }
}

/** 仅切换本局世界书条目启用状态，保留正文、关键词与注入路由。 */
export async function setCampaignWorldInfoEnabled(campaignId, entryIndex, enabled) {
  if (isTauri()) {
    return await invoke('set_campaign_world_info_enabled', { campaignId, entryIndex, enabled: !!enabled })
  }
}

/** 删除本局世界书条目 */
export async function deleteCampaignWorldInfoEntry(campaignId, entryIndex) {
  if (isTauri()) {
    return await invoke('delete_campaign_world_info_entry', { campaignId, entryIndex })
  }
}

/** 设置本局世界书路由 */
export async function setCampaignWorldInfoRoute(campaignId, entryIndex, route) {
  if (isTauri()) {
    return await invoke('set_campaign_world_info_route', { campaignId, entryIndex, route })
  }
}

/** 卡模板世界书只读 */
export async function getCharacterWorldInfo(characterId) {
  if (isTauri()) {
    return await invoke('get_character_world_info', { characterId })
  }
  return { campaign_id: `card:${characterId}`, entry_count: 0, constant_count: 0, selective_count: 0, entries: [] }
}

/** 卡模板世界书单条完整正文 */
export async function getCharacterWorldInfoEntry(characterId, entryIndex) {
  if (isTauri()) {
    return await invoke('get_character_world_info_entry', { characterId, entryIndex })
  }
  return null
}

/** 本局世界书单条完整正文 */
export async function getCampaignWorldInfoEntry(campaignId, entryIndex) {
  if (isTauri()) {
    return await invoke('get_campaign_world_info_entry', { campaignId, entryIndex })
  }
  return null
}


// ─── Card Shell（可见壳 + 宿主代持）────────────────────────────────────────

/** 从角色卡 extensions/regex/tavern_helper 提取壳清单 */
export async function getCardShellManifest(characterId) {
  if (isTauri()) {
    return await invoke('get_card_shell_manifest', { characterId })
  }
  return {
    character_id: characterId,
    shells: [],
    remote_urls: [],
    opening_home_url: null,
    opening_custom_url: null,
    status_bar_url: null,
    tavern_helper_count: 0,
  }
}

export async function cardShellListAllowedHosts() {
  if (isTauri()) {
    return await invoke('card_shell_list_allowed_hosts')
  }
  return []
}

export async function cardShellAllowHost(host) {
  if (isTauri()) {
    return await invoke('card_shell_allow_host', { host })
  }
}

/** 清空卡壳磁盘缓存（L6 刷新通道）；返回清掉的对象数 */
export async function cardShellClearCache() {
  if (isTauri()) {
    return await invoke('card_shell_clear_cache')
  }
  return 0
}

/** V4 存储健康报告：blocking=true 的事件表示文件损坏且写栅栏生效（启动拦截用） */
export async function storageHealthReport() {
  if (isTauri()) {
    return await invoke('storage_health_report')
  }
  return []
}

/** V4 用户确认损坏文件「从空白开始」→ 解除该路径写栅栏 */
export async function storageHealthAcknowledge(path) {
  if (isTauri()) {
    return await invoke('storage_health_acknowledge', { path })
  }
  return false
}

/** 宿主代持拉取远程壳资源；失败抛错（不静默降级） */
export async function cardShellFetchUrl(url) {
  if (isTauri()) {
    return await invoke('card_shell_fetch_url', { url })
  }
  throw new Error('card shell fetch requires Tauri host')
}

/** 按需取 deferred 的 TH inline 大脚本 */
export async function getCardShellInlineJs(characterId, label) {
  if (isTauri()) {
    return await invoke('get_card_shell_inline_js', { characterId, label })
  }
  return ''
}


/** 获取当前活跃 Campaign */
export async function getActiveCampaign() {
  if (isTauri()) {
    return await invoke('get_active_campaign')
  }
  return null
}

/** 列出 Campaign 内的角色实例 */
export async function listInstances(campaignId) {
  if (isTauri()) {
    return await invoke('list_instances', { campaignId })
  }
  return []
}

/** 获取单个角色实例详情 */
export async function getInstance(campaignId, instanceId) {
  if (isTauri()) {
    return await invoke('get_instance', { campaignId, instanceId })
  }
  return null
}

/** 查角色实例的当前变量值 */
export async function getCharacterVariables(campaignId, instanceId) {
  if (isTauri()) {
    return await invoke('get_character_variables', { campaignId, instanceId })
  }
  return []
}

/** 手动改角色实例变量值 */
export async function setCharacterVariable(campaignId, instanceId, key, value) {
  if (isTauri()) {
    return await invoke('set_character_variable', { campaignId, instanceId, key, value })
  }
}

/** 查 Campaign 全局变量 */
export async function getCampaignVariables(campaignId) {
  if (isTauri()) {
    return await invoke('get_campaign_variables', { campaignId })
  }
  return []
}

/** 改 Campaign 全局变量 */
export async function setCampaignVariable(campaignId, key, value) {
  if (isTauri()) {
    return await invoke('set_campaign_variable', { campaignId, key, value })
  }
}

/** 临场角色升级为常驻 */
export async function promoteTemporaryInstance(campaignId, instanceId) {
  if (isTauri()) {
    return await invoke('promote_temporary_instance', { campaignId, instanceId })
  }
}

// ─── P2 知识 / 任务 / 摘要 ──────────────────────────────────────────────────

/** 列角色可见信息（不传 characterId 则返回整个 campaign 所有角色的知识） */
export async function listCharacterKnowledge(campaignId, characterId = null) {
  if (isTauri()) {
    return await invoke('list_character_knowledge', { campaignId, characterId })
  }
  return []
}

/** 列叙事计划任务（可按状态筛：pending/active/likely_completed/completed/abandoned） */
export async function listTasks(campaignId, statusFilter = null) {
  if (isTauri()) {
    return await invoke('list_tasks', { campaignId, statusFilter })
  }
  return []
}

/**
 * 用户手动建任务（伏笔/目标）
 * @param {string} campaignId
 * @param {string} title
 * @param {string} description
 * @param {Array} triggers - [{Event: "描述"}, {TurnReminder: 30}, {StoryTime: "第2年6月"}, "Manual"]
 */
export async function createTask(campaignId, title, description, triggers) {
  if (isTauri()) {
    return await invoke('create_task', { campaignId, title, description, triggers })
  }
  return 'mock-task-id'
}

/** 标记任务完成 */
export async function completeTask(taskId) {
  if (isTauri()) {
    return await invoke('complete_task', { taskId })
  }
}

/** 放弃任务 */
export async function abandonTask(taskId) {
  if (isTauri()) {
    return await invoke('abandon_task', { taskId })
  }
}

/** 列本轮剧情摘要（按 turn 升序） */
export async function listRoundSummaries(campaignId) {
  if (isTauri()) {
    return await invoke('list_round_summaries', { campaignId })
  }
  return []
}

/**
 * 读取当前 Campaign 活动 Turn 的质量门禁结果（刷新后回填用）。
 * @returns {Promise<null | {
 *   turn_id: string,
 *   attempt_id: string,
 *   status: string,
 *   passed: boolean,
 *   warning_count: number,
 *   error_count: number,
 *   warnings: string[],
 * }>}
 */
export async function getActiveTurnQuality(campaignId) {
  if (isTauri()) {
    return await invoke('get_active_turn_quality', { campaignId })
  }
  return null
}

/** 读取 Campaign 当前草稿的 Accept-before 记账小票。 */
export async function getActiveTurnReceipt(campaignId, nodeId) {
  if (isTauri()) {
    return await invoke('get_active_turn_receipt', { campaignId, nodeId })
  }
  return null
}

/** 仅重跑当前草稿的 Summarizer + PostProcessor，并返回新小票。 */
export async function retryActiveTurnPostprocess(campaignId, nodeId) {
  if (isTauri()) {
    return await invoke('retry_active_turn_postprocess', { campaignId, nodeId })
  }
  return null
}

// ─── P3 Meta Agent / MVU 五合一 / ST 预设分类 ──────────────────────────────

/** 开始一个新的 Meta 对话（返回 conversation_id） */
export async function metaStartConversation() {
  if (isTauri()) {
    return await invoke('meta_start_conversation')
  }
  return 'mock-meta-conv-1'
}

/** 跑一轮 Meta 对话 */
/**
 * 跑一轮 Meta 对话（流式）
 * @param {string} conversationId
 * @param {string} userInput
 * @param {(delta: string) => void} [onProgress] token 增量回调
 */
export async function metaChat(conversationId, userInput, onProgress) {
  if (isTauri()) {
    const { Channel } = await import('@tauri-apps/api/core')
    const channel = new Channel()
    channel.onmessage = (event) => {
      // event: { event_type: 'meta_progress', data: { delta } }
      if (onProgress && event?.event_type === 'meta_progress') {
        onProgress(event.data?.delta ?? '')
      }
    }
    return await invoke('meta_chat', { conversationId, userInput, onEvent: channel })
  }
  return {
    conversation_id: conversationId,
    agent_message: { role: 'agent', content: '（mock）我看了下配置，没发现明显问题。' },
    new_patch: null,
  }
}

/** 获取 Meta 对话完整历史 */
export async function metaGetConversation(conversationId) {
  if (isTauri()) {
    return await invoke('meta_get_conversation', { conversationId })
  }
  return null
}

/** 列所有待采纳的 Meta Patch */
export async function metaListPendingPatches() {
  if (isTauri()) {
    return await invoke('meta_list_pending_patches')
  }
  return []
}

/** 忽略一个 Meta Patch */
export async function metaDismissPatch(patchId) {
  if (isTauri()) {
    return await invoke('meta_dismiss_patch', { patchId })
  }
}

/** 手动触发 MVU 五合一分析（D44：手动按钮） */
export async function metaAnalyzeMvuCard(sourceCharacterId) {
  if (isTauri()) {
    return await invoke('meta_analyze_mvu_card', { sourceCharacterId })
  }
  return null
}

/** 列所有已分析的 MVU 翻译 */
export async function metaListMvuTranslations() {
  if (isTauri()) {
    return await invoke('meta_list_mvu_translations')
  }
  return []
}

/** 查某角色卡的 MVU 翻译详情（前端渲染状态栏用） */
export async function metaGetMvuTranslation(sourceCharacterId) {
  if (isTauri()) {
    return await invoke('meta_get_mvu_translation', { sourceCharacterId })
  }
  return null
}

/** 预览 MVU schema 合并结果（每个 definition 一条 preview） */
export async function metaPreviewMvuApply(sourceCharacterId) {
  if (isTauri()) {
    return await invoke('meta_preview_mvu_apply', { sourceCharacterId })
  }
  return []
}

/** 应用 MVU schema 到指定 definition（写盘 + backfill instance） */
export async function metaApplyMvuSchema(sourceCharacterId, definitionId) {
  if (isTauri()) {
    return await invoke('meta_apply_mvu_schema', { sourceCharacterId, definitionId })
  }
}

/** 手动触发 ST 预设 LLM 分类 */
export async function metaClassifyStPreset(presetId) {
  if (isTauri()) {
    return await invoke('meta_classify_st_preset', { presetId })
  }
  return null
}

/** Campaign 健康检查（确定性数据校验，零 LLM） */
export async function metaHealthCheck(campaignId) {
  if (isTauri()) {
    return await invoke('meta_health_check', { campaignId })
  }
  return []
}

// ─── 类型化 Patch（第三轮：修复建议闭环）────────────────────────────────────

/** 为 Campaign 的 health issues 生成修复方案 */
export async function metaProposeCampaignRepairs(campaignId) {
  if (isTauri()) {
    return await invoke('meta_propose_campaign_repairs', { campaignId })
  }
  return []
}

/** 列出所有已生成的类型化 patch */
export async function metaListTypedPatches() {
  if (isTauri()) {
    return await invoke('meta_list_typed_patches')
  }
  return []
}

/** 预览一条 patch（含 stale 检测 + diff） */
export async function metaPreviewTypedPatch(patchId, campaignId) {
  if (isTauri()) {
    return await invoke('meta_preview_typed_patch', { patchId, campaignId })
  }
  return { stale: false, patch: null, diff: [] }
}

/** 接受一条 patch（写入 CampaignStore） */
export async function metaAcceptTypedPatch(patchId, campaignId) {
  if (isTauri()) {
    return await invoke('meta_accept_typed_patch', { patchId, campaignId })
  }
}

/** 忽略一条 patch */
export async function metaDismissTypedPatch(patchId) {
  if (isTauri()) {
    return await invoke('meta_dismiss_typed_patch', { patchId })
  }
}

// ─── 生成溯源（命令已存在于后端）─────────────────────────────────────────────

/** 解释某条消息的生成溯源 */
export async function metaExplainGeneration(conversationId, nodeId) {
  if (isTauri()) {
    return await invoke('meta_explain_generation', { conversationId, nodeId })
  }
  return null
}

// ─── W7 导出命令 ─────────────────────────────────────────────────────────────

/** 导出单个角色卡为 ST PNG（含 tEXt "chara" 块） */
export async function exportStCardPng(characterId) {
  if (isTauri()) {
    return await invoke('export_st_card_png', { characterId })
  }
  return null
}

/**
 * 导出 Campaign 全部角色为 ST PNG + 共享 lorebook
 * 返回 { cards: [{filename, data}], lorebook_json }
 */
export async function exportCampaignStCards(campaignId) {
  if (isTauri()) {
    return await invoke('export_campaign_st_cards', { campaignId })
  }
  return { cards: [], lorebook_json: '{}' }
}

/** 导出 StoryForge Campaign 完整 JSON Bundle */
export async function exportCampaignBundle(campaignId) {
  if (isTauri()) {
    return await invoke('export_campaign_bundle', { campaignId })
  }
  return null
}

/** 导入 StoryForge Campaign JSON Bundle */
export async function importCampaignBundle(bundleJson) {
  if (isTauri()) {
    return await invoke('import_campaign_bundle', { bundleJson })
  }
  return null
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

// ─── W8 MVU JS Runtime API ───────────────────────────────────────────────

/**
 * MVU WebView 运行时状态
 *
 * 实际可用性由 Rust 侧 WebViewMvuRuntime.is_available() 报告（始终 true）。
 * 前端侧通过 MvuJsRuntime.vue 的 iframe 就绪状态判断。
 * 此函数作为前端 API 入口供其他模块查询。
 */
export function getMvuRuntimeStatus() {
  // 通过 DOM 查询 MvuJsRuntime 的 iframe 是否已加载
  // 实际使用中，上层代码通过 listen('mvu:execute') 等事件直接通信
  return { available: true, note: 'WebView runtime (iframe sandbox)' }
}

