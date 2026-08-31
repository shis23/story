// 迁移自 App.vue:623-658 的 handleImport。
// 消费 campaignStore: activeChar / activeCharDetail / currentConversationId。
// 消费 writingStore: selectedGreetingIndex（loadCharDetail 内重置为 0）。
// 消费 uiStore: importError。
//
// 范围外依赖（注入）：
// - broadcastPluginEvent(event, data)：插件事件广播（App.vue 侧持有）
// - applySelectedOpeningMessage()：来自 useGreeting，导入后用角色开场白替换消息列表
// - normalizeGreetingSelection()：来自 useGreeting，loadCharDetail 内重置开场白索引后归一
// 未传时安全降级为 no-op。

import { useCampaignStore } from '../stores/campaign.js'
import { useWritingStore } from '../stores/writing.js'
import { useUiStore } from '../stores/ui.js'
import {
  importCharacter,
  extractCharacters,
  getCharacter,
} from '../tauri-api.js'
import { ST_EVENT_TYPES } from '../plugin-bridge.js'
import { errorText } from '../utils/errorText.js'

/**
 * @param {{
 *   broadcastPluginEvent?: (event: string, data?: object) => void,
 *   applySelectedOpeningMessage?: () => void,
 *   normalizeGreetingSelection?: () => void,
 * }} [options]
 */
export function useCharacterImport(options = {}) {
  const campaignStore = useCampaignStore()
  const writingStore = useWritingStore()
  const uiStore = useUiStore()

  const broadcastPluginEvent = options.broadcastPluginEvent || (() => {})
  const applySelectedOpeningMessage = options.applySelectedOpeningMessage || (() => {})
  const normalizeGreetingSelection = options.normalizeGreetingSelection || (() => {})

  // 来源 App.vue:661-669 loadCharDetail（未列入迁移清单，但 handleImport 依赖它；
  // 此处内联其逻辑：getCharacter + 更新 store + 归一化开场白索引）。
  async function loadCharDetail(id) {
    try {
      campaignStore.activeCharDetail = await getCharacter(id)
      writingStore.selectedGreetingIndex = 0
      normalizeGreetingSelection()
    } catch (e) {
      console.error('加载详情失败:', e)
    }
  }

  // 来源 App.vue:623-658 handleImport
  async function handleImport() {
    uiStore.importError = ''
    try {
      const { open } = await import('@tauri-apps/plugin-dialog')
      const filePath = await open({
        multiple: false,
        filters: [{ name: '角色卡', extensions: ['png', 'json'] }],
      })
      if (!filePath) return

      const { readFile } = await import('@tauri-apps/plugin-fs')
      const data = await readFile(filePath)
      const result = await importCharacter(data)

      // 导入成功，先设为当前活跃角色并加载详情（让用户立即看到导入结果）
      campaignStore.activeChar = result
      campaignStore.currentConversationId = null
      await loadCharDetail(result.id)
      broadcastPluginEvent(ST_EVENT_TYPES.CHARACTER_LOADED, {
        characterId: result.id,
        name: result.name,
      })

      // 自动触发角色识别（写入 CampaignStore.cards.json，供 Campaign 面板使用）。
      // 带 UI 进度反馈：抽取期间 uiStore.extracting 显示「正在识别角色…」。
      // 失败不阻塞导入主流程——识别失败时 Campaign 面板可手动重试。
      uiStore.extracting = { id: result.id, name: result.name }
      try {
        await extractCharacters(result.id)
        // 抽取成功后刷新详情，让角色定义立即可见
        await loadCharDetail(result.id)
        applySelectedOpeningMessage()
      } catch (e) {
        console.error('角色识别失败（不影响导入，可在 Campaign 面板重试）:', e)
        uiStore.importError = '角色识别失败（可在 Campaign 面板重试）: ' + errorText(e)
      } finally {
        uiStore.extracting = null
      }
    } catch (err) {
      uiStore.importError = errorText(err)
    }
  }

  return { handleImport }
}
