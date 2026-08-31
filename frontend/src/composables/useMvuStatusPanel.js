// MVU 原生状态面板装配 + interactions 分发（写作面 after-messages 槽）
//
// 数据链：activeCampaign.card_id → getCard → source_character_id →
//   metaGetMvuTranslation（ui_bindings + interactions）
//   + listInstances（DTO 自带 variables）+ getCampaignVariables（campaign 级变量打底）
// 刷新时机：campaign 切换；postprocess running→done（后处理可能改了变量）。
// 分发：planMvuInteraction 出纯计划 → persistShellVariableWrite 写变量
//   （单实例 → instance 作用域，否则 campaign）→ trigger_next_turn 走 startWriting。
// API 可经 options.*Api 注入以便 node --test 直测；竞态用单调 token 守卫。
import { ref, watch } from 'vue'
import { useCampaignStore } from '../stores/campaign.js'
import { useWritingStore } from '../stores/writing.js'
import {
  getCard as apiGetCard,
  getCampaignVariables as apiGetCampaignVariables,
  listInstances as apiListInstances,
  logAppendFrontend,
  metaGetMvuTranslation as apiMetaGetMvuTranslation,
  setCampaignVariable as apiSetCampaignVariable,
  setCharacterVariable as apiSetCharacterVariable,
} from '../tauri-api.js'
import { buildCampaignMvuStatusSections } from '../utils/campaignMvuStatusBar.js'
import { planMvuInteraction } from '../utils/mvuInteractions.js'
import { persistShellVariableWrite } from '../utils/shellVariableOutbox.js'
import { errorText } from '../utils/errorText.js'

export function useMvuStatusPanel(options = {}) {
  const campaign = useCampaignStore()
  const writing = useWritingStore()
  const getCardApi = options.getCardApi || apiGetCard
  const metaGetMvuTranslationApi = options.metaGetMvuTranslationApi || apiMetaGetMvuTranslation
  const listInstancesApi = options.listInstancesApi || apiListInstances
  const getCampaignVariablesApi = options.getCampaignVariablesApi || apiGetCampaignVariables
  const setCampaignVariableApi = options.setCampaignVariableApi || apiSetCampaignVariable
  const setCharacterVariableApi = options.setCharacterVariableApi || apiSetCharacterVariable
  const persistWriteApi = options.persistWriteApi || persistShellVariableWrite
  const startWriting = options.startWriting || null
  const logApi = options.logApi || logAppendFrontend
  const alertDialog = options.alertDialog || null

  const mvuStatusSections = ref([])
  const mvuInteractionMappings = ref([])
  const mvuInteractionBusy = ref(false)
  let loadToken = 0

  async function refreshMvuStatusPanel() {
    const token = ++loadToken
    const campaignId = campaign.activeCampaign?.id
    const cardId = campaign.activeCampaign?.card_id
    if (!campaignId || !cardId) {
      mvuStatusSections.value = []
      mvuInteractionMappings.value = []
      return
    }
    try {
      const card = await getCardApi(cardId)
      const sourceCharacterId = card?.source_character_id
      const translationDetail = sourceCharacterId
        ? await metaGetMvuTranslationApi(sourceCharacterId)
        : null
      if (!translationDetail) {
        if (token === loadToken) {
          mvuStatusSections.value = []
          mvuInteractionMappings.value = []
        }
        return
      }
      const [instances, campaignVariables] = await Promise.all([
        listInstancesApi(campaignId),
        getCampaignVariablesApi(campaignId),
      ])
      if (token !== loadToken) return
      mvuStatusSections.value = buildCampaignMvuStatusSections({
        card,
        translationDetail,
        instances,
        campaignVariables,
      })
      const interactions = translationDetail.translation?.interactions
      mvuInteractionMappings.value = Array.isArray(interactions) ? interactions : []
    } catch (e) {
      console.error('refreshMvuStatusPanel:', e)
      if (token === loadToken) {
        mvuStatusSections.value = []
        mvuInteractionMappings.value = []
      }
    }
  }

  /**
   * 执行一个 InteractionMapping：变量写入（先）→ 面板刷新 → trigger_next_turn（后）。
   * 返回 { plan, results }；无活跃 campaign 或并发点击时返回 null。
   * 写入目标：恰好一个卡绑定实例 → 该实例作用域；否则 campaign 作用域。
   */
  async function dispatchMvuInteraction(mapping) {
    if (mvuInteractionBusy.value) return null
    const campaignId = campaign.activeCampaign?.id
    if (!campaignId || !mapping) return null

    const sections = mvuStatusSections.value
    const variables = sections[0]?.mvuState?.variables || []
    // 与 AppV2.onShellVarWrite 同判据（含回退）：卡绑定实例恰一个 → 该实例；
    // 否则全 campaign 单实例回退——两条写入路径的作用域目标必须一致。
    const nameMapIds = Object.keys(campaign.instanceNameMap || {})
    const singleInstanceId =
      (sections.length === 1 ? sections[0].instanceId : null) ||
      (nameMapIds.length === 1 ? nameMapIds[0] : null)
    // 模板键（{角色名} 段）按目标实例名展开
    const singleInstanceName =
      (sections.length === 1 ? sections[0].instanceName : '') ||
      (singleInstanceId ? campaign.instanceNameMap?.[singleInstanceId] || '' : '')
    const plan = planMvuInteraction(mapping, { variables, instanceName: singleInstanceName })

    mvuInteractionBusy.value = true
    const results = []
    try {
      for (const write of plan.writes) {
        const key = singleInstanceId
          ? `instance:${singleInstanceId}:${write.key}`
          : write.key
        results.push(
          await persistWriteApi({
            campaignId,
            instanceId: singleInstanceId,
            key,
            value: write.value,
            setCampaignVariable: setCampaignVariableApi,
            setCharacterVariable: setCharacterVariableApi,
            log: logApi,
          }),
        )
      }
      if (plan.writes.length) await refreshMvuStatusPanel()
      // 变量写入失败时不得继续 trigger_next_turn：下一轮会基于旧变量生成，
      // 且失败本身必须可见（此前 results 被整体忽略，点击按钮静默无效）。
      const failed = results.filter((r) => r && r.ok === false)
      if (failed.length) {
        const detail = failed
          .map((r) => `${r.key}: ${errorText(r.error)}`)
          .join('\n')
        const message = `交互已触发，但 ${failed.length} 项变量写入失败（已跳过下一轮生成）:\n${detail}`
        console.error('dispatchMvuInteraction:', message)
        if (alertDialog) await alertDialog(message)
        else await logApi('error', message)
        return { plan, results }
      }
      for (const hint of plan.hints) {
        if (typeof startWriting === 'function' && !writing.isWriting) {
          await startWriting(hint)
        }
      }
    } finally {
      mvuInteractionBusy.value = false
    }
    return { plan, results }
  }

  watch(
    () => campaign.activeCampaign?.id,
    () => {
      refreshMvuStatusPanel()
    },
    { immediate: true },
  )
  // running→done 的沿触发（跳过与初始状态无关的赋值），拉后处理写回的新变量值
  watch(
    () => writing.pipeline.postprocess.status,
    (status, prev) => {
      if (status === 'done' && prev === 'running') refreshMvuStatusPanel()
    },
  )

  return {
    mvuStatusSections,
    mvuInteractionMappings,
    mvuInteractionBusy,
    dispatchMvuInteraction,
    refreshMvuStatusPanel,
  }
}
