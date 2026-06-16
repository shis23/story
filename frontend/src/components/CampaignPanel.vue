<script setup>
import { ref, onMounted, computed } from 'vue'
import {
  listCards, getCard, extractCharacters,
  listCampaigns, createCampaign, setActiveCampaign, getActiveCampaign,
  listInstances, getCharacterVariables, setCharacterVariable,
  listCharacterKnowledge, listTasks, createTask, completeTask, abandonTask,
  listRoundSummaries
} from '../tauri-api.js'

const emit = defineEmits(['close', 'campaign-changed'])

// ─── Tab 控制 ───
const activeTab = ref('cards') // 'cards' | 'campaigns' | 'detail'

// ─── Cards 状态 ───
const cards = ref([])
const loadingCards = ref(false)
const extractingCardId = ref(null) // 正在识别的卡 ID
const expandedCardId = ref(null)
const cardDetail = ref(null)

// ─── Campaigns 状态 ───
const selectedCardId = ref(null)
const campaigns = ref([])
const loadingCampaigns = ref(false)
const activeCampaign = ref(null)
const showNewCampaign = ref(false)
const newCampaignName = ref('')
const creatingCampaign = ref(false)

// ─── Detail 状态 ───
const selectedCampaignId = ref(null)
const instances = ref([])
const knowledge = ref([])
const tasks = ref([])
const summaries = ref([])
const expandedInstanceId = ref(null)
const instanceVariables = ref([])

// ─── 初始化 ───
onMounted(async () => {
  await refreshCards()
  activeCampaign.value = await getActiveCampaign()
})

// ─── Cards 操作 ───
async function refreshCards() {
  loadingCards.value = true
  try {
    cards.value = await listCards()
  } finally {
    loadingCards.value = false
  }
}

async function handleExtract(card) {
  extractingCardId.value = card.source_character_id
  try {
    const result = await extractCharacters(card.source_character_id)
    await refreshCards()
    // 自动展开刚识别的卡
    expandedCardId.value = result.id
    cardDetail.value = await getCard(result.id)
  } catch (e) {
    alert('角色识别失败: ' + e)
  } finally {
    extractingCardId.value = null
  }
}

async function toggleCard(card) {
  if (expandedCardId.value === card.id) {
    expandedCardId.value = null
    cardDetail.value = null
  } else {
    expandedCardId.value = card.id
    cardDetail.value = await getCard(card.id)
  }
}

// ─── Campaigns 操作 ───
async function refreshCampaigns() {
  if (!selectedCardId.value) return
  loadingCampaigns.value = true
  try {
    campaigns.value = await listCampaigns(selectedCardId.value)
  } finally {
    loadingCampaigns.value = false
  }
}

async function handleCreateCampaign() {
  if (!newCampaignName.value.trim() || !selectedCardId.value) return
  creatingCampaign.value = true
  try {
    const result = await createCampaign(selectedCardId.value, newCampaignName.value.trim())
    newCampaignName.value = ''
    showNewCampaign.value = false
    await refreshCampaigns()
    // 自动设为活跃
    await handleSetActive(result.id)
  } catch (e) {
    alert('创建失败: ' + e)
  } finally {
    creatingCampaign.value = false
  }
}

async function handleSetActive(campaignId) {
  await setActiveCampaign(campaignId)
  activeCampaign.value = await getActiveCampaign()
  emit('campaign-changed', activeCampaign.value)
}

async function openCampaignDetail(campaignId) {
  selectedCampaignId.value = campaignId
  activeTab.value = 'detail'
  await refreshDetail()
}

// ─── Detail 操作 ───
const detailSubTab = ref('instances') // 'instances' | 'knowledge' | 'tasks' | 'summaries'

async function refreshDetail() {
  if (!selectedCampaignId.value) return
  const cid = selectedCampaignId.value
  instances.value = await listInstances(cid)
  knowledge.value = await listCharacterKnowledge(cid)
  tasks.value = await listTasks(cid)
  summaries.value = await listRoundSummaries(cid)
}

async function toggleInstance(inst) {
  if (expandedInstanceId.value === inst.id) {
    expandedInstanceId.value = null
    instanceVariables.value = []
  } else {
    expandedInstanceId.value = inst.id
    instanceVariables.value = await getCharacterVariables(selectedCampaignId.value, inst.id)
  }
}

async function handleVariableChange(instanceId, key, value) {
  try {
    // 仅解析布尔字面量；数字/字符串保持原样，由后端 schema 决定类型。
    // 历史 bug：启发式 Number(value) 会把纯数字字符串（如电话号 "13800001111"）
    // 强转 Number 丢失前导零/精度，或把 "0x1F"/"1e3" 这种字面量悄悄转换。
    let parsed = value
    if (value === 'true') parsed = true
    else if (value === 'false') parsed = false

    await setCharacterVariable(selectedCampaignId.value, instanceId, key, parsed)
    // 刷新变量
    instanceVariables.value = await getCharacterVariables(selectedCampaignId.value, instanceId)
  } catch (e) {
    alert('设置变量失败: ' + e)
  }
}

// ─── 任务操作 ───
const showNewTask = ref(false)
const newTaskTitle = ref('')
const newTaskDesc = ref('')

async function handleCreateTask() {
  if (!newTaskTitle.value.trim()) return
  try {
    await createTask(selectedCampaignId.value, newTaskTitle.value.trim(), newTaskDesc.value.trim(), ['Manual'])
    newTaskTitle.value = ''
    newTaskDesc.value = ''
    showNewTask.value = false
    tasks.value = await listTasks(selectedCampaignId.value)
  } catch (e) {
    alert('创建任务失败: ' + e)
  }
}

async function handleCompleteTask(taskId) {
  await completeTask(taskId)
  tasks.value = await listTasks(selectedCampaignId.value)
}

async function handleAbandonTask(taskId) {
  if (!confirm('确定放弃该任务？')) return
  await abandonTask(taskId)
  tasks.value = await listTasks(selectedCampaignId.value)
}

function taskStatusText(status) {
  if (typeof status === 'string') return status
  if (status?.likely_completed != null) return `likely (${Math.round(status.likely_completed * 100)}%)`
  return JSON.stringify(status)
}

function taskStatusClass(status) {
  const s = typeof status === 'string' ? status : ''
  if (s === 'pending') return 'bg-wait/10 text-wait'
  if (s === 'active') return 'bg-running/10 text-running'
  if (s === 'completed') return 'bg-ok/10 text-ok'
  if (s === 'abandoned') return 'bg-ink-soft/10 text-ink-soft'
  return 'bg-warn/10 text-warn'
}

function knowledgeSourceText(source) {
  const map = { witnessed: '👁 亲眼', told_by_other: '💬 被告知', inferred: '🔮 推断', backstory: '📖 背景' }
  return map[source] || source
}
</script>

<template>
  <!-- 弹层外壳 -->
  <div class="fixed inset-0 z-50 bg-black/40 backdrop-blur-sm flex items-end sm:items-center justify-center" @click.self="emit('close')">
    <div class="bg-bg w-full max-w-lg max-h-[90vh] overflow-hidden rounded-t-2xl sm:rounded-2xl border border-line flex flex-col">

      <!-- 顶栏 -->
      <div class="sticky top-0 z-10 bg-bg border-b border-line px-4 py-3 flex items-center justify-between shrink-0">
        <button @click="emit('close')" class="text-ink-soft hover:text-ink text-sm">← 返回</button>
        <span class="font-medium text-ink text-sm">Campaign 管理</span>
        <div class="w-12"></div>
      </div>

      <!-- Tab 切换 -->
      <div class="flex border-b border-line shrink-0">
        <button
          v-for="tab in [{key:'cards',label:'角色卡'},{key:'campaigns',label:'游玩档'},{key:'detail',label:'档详情'}]"
          :key="tab.key"
          @click="activeTab = tab.key"
          class="flex-1 py-2.5 text-xs font-medium transition-colors"
          :class="activeTab === tab.key ? 'text-accent border-b-2 border-accent' : 'text-ink-soft'"
        >{{ tab.label }}</button>
      </div>

      <!-- 内容区 -->
      <div class="flex-1 overflow-y-auto p-4 space-y-3">

        <!-- ═══ Tab 1: 角色卡 ═══ -->
        <template v-if="activeTab === 'cards'">
          <div v-if="loadingCards" class="text-center text-ink-soft text-sm py-8">加载中…</div>

          <div v-else-if="cards.length === 0" class="text-center text-ink-soft text-sm py-8">
            还没有角色卡，请先导入
          </div>

          <div v-for="card in cards" :key="card.id" class="bg-surface rounded-xl border border-line overflow-hidden">
            <!-- 卡头部 -->
            <div
              class="flex items-center gap-3 px-3 py-2.5 cursor-pointer hover:bg-bg transition-colors"
              @click="toggleCard(card)"
            >
              <div class="flex-1 min-w-0">
                <div class="text-sm font-medium text-ink truncate">{{ card.name }}</div>
                <div class="text-xs text-ink-soft">
                  {{ card.definition_count }} 个角色定义
                  <span v-if="card.extracted" class="text-ok ml-1">✓ 已识别</span>
                  <span v-else class="text-warn ml-1">未识别</span>
                </div>
              </div>
              <button
                v-if="!card.extracted"
                @click.stop="handleExtract(card)"
                :disabled="extractingCardId === card.source_character_id"
                class="px-2.5 py-1 rounded-full text-xs font-medium bg-accent text-white disabled:opacity-50"
              >
                {{ extractingCardId === card.source_character_id ? '识别中…' : '识别角色' }}
              </button>
              <span class="text-ink-soft text-xs">{{ expandedCardId === card.id ? '▲' : '▼' }}</span>
            </div>

            <!-- 卡展开详情 -->
            <div v-if="expandedCardId === card.id && cardDetail" class="border-t border-line px-3 py-2 space-y-2">
              <div v-if="cardDetail.character_definitions.length === 0" class="text-xs text-ink-soft py-2">
                暂无角色定义，请点击「识别角色」
              </div>
              <div
                v-for="def in cardDetail.character_definitions"
                :key="def.id"
                class="bg-bg rounded-lg px-3 py-2 text-xs"
              >
                <div class="flex items-center gap-2 mb-1">
                  <span class="font-medium text-ink">{{ def.name }}</span>
                  <span class="px-1.5 py-0.5 rounded text-[10px]"
                    :class="{
                      'bg-accent/10 text-accent': def.role_type === 'Protagonist',
                      'bg-ok/10 text-ok': def.role_type === 'Supporting',
                      'bg-ink-soft/10 text-ink-soft': def.role_type === 'Extra',
                    }"
                  >{{ def.role_type }}</span>
                  <span v-if="def.group" class="text-ink-soft">{{ def.group }}</span>
                </div>
                <div class="text-ink-soft line-clamp-2">{{ def.persona_prompt }}</div>
              </div>

              <!-- 开档按钮 -->
              <div v-if="cardDetail.character_definitions.length > 0" class="pt-1">
                <button
                  @click="selectedCardId = card.id; activeTab = 'campaigns'; refreshCampaigns()"
                  class="w-full py-2 rounded-lg text-xs font-medium bg-accent text-white"
                >管理游玩档 →</button>
              </div>
            </div>
          </div>
        </template>

        <!-- ═══ Tab 2: 游玩档 ═══ -->
        <template v-if="activeTab === 'campaigns'">
          <!-- 未选卡时提示选卡 -->
          <div v-if="!selectedCardId" class="text-center text-ink-soft text-sm py-8">
            请先在「角色卡」tab 选择一张卡
          </div>

          <template v-else>
            <div v-if="loadingCampaigns" class="text-center text-ink-soft text-sm py-8">加载中…</div>

            <div v-else-if="campaigns.length === 0 && !showNewCampaign" class="text-center py-8">
              <div class="text-ink-soft text-sm mb-3">还没有游玩档</div>
              <button @click="showNewCampaign = true" class="px-4 py-2 rounded-lg text-xs font-medium bg-accent text-white">
                新建游玩档
              </button>
            </div>

            <!-- 新建表单 -->
            <div v-if="showNewCampaign" class="bg-surface rounded-xl border border-line p-3 space-y-2">
              <div class="text-xs font-medium text-ink">新建游玩档</div>
              <input
                v-model="newCampaignName"
                placeholder="输入档名（如：第一周目）"
                class="w-full px-3 py-2 text-sm rounded-lg border border-line bg-bg focus:outline-none focus:border-accent"
                @keyup.enter="handleCreateCampaign"
              />
              <div class="flex gap-2">
                <button @click="showNewCampaign = false" class="flex-1 py-1.5 rounded-lg text-xs bg-bg text-ink-soft">取消</button>
                <button
                  @click="handleCreateCampaign"
                  :disabled="creatingCampaign || !newCampaignName.trim()"
                  class="flex-1 py-1.5 rounded-lg text-xs font-medium bg-accent text-white disabled:opacity-50"
                >{{ creatingCampaign ? '创建中…' : '创建' }}</button>
              </div>
            </div>

            <!-- Campaign 列表 -->
            <div
              v-for="camp in campaigns"
              :key="camp.id"
              class="bg-surface rounded-xl border overflow-hidden cursor-pointer transition-colors"
              :class="activeCampaign?.id === camp.id ? 'border-accent' : 'border-line hover:border-accent-border'"
              @click="openCampaignDetail(camp.id)"
            >
              <div class="flex items-center gap-3 px-3 py-2.5">
                <div class="flex-1 min-w-0">
                  <div class="flex items-center gap-2">
                    <span class="text-sm font-medium text-ink">{{ camp.name }}</span>
                    <span v-if="activeCampaign?.id === camp.id" class="px-1.5 py-0.5 rounded text-[10px] bg-accent/10 text-accent">活跃</span>
                  </div>
                  <div class="text-xs text-ink-soft">{{ camp.instance_count }} 个角色实例</div>
                </div>
                <button
                  v-if="activeCampaign?.id !== camp.id"
                  @click.stop="handleSetActive(camp.id)"
                  class="px-2 py-1 rounded-full text-[10px] bg-bg text-ink-soft hover:bg-line"
                >设为活跃</button>
                <span class="text-ink-soft text-xs">→</span>
              </div>
            </div>

            <!-- 新建按钮（有档时显示） -->
            <button
              v-if="campaigns.length > 0 && !showNewCampaign"
              @click="showNewCampaign = true"
              class="w-full py-2 rounded-lg text-xs font-medium bg-bg text-ink-soft hover:bg-line border border-dashed border-line"
            >+ 新建游玩档</button>
          </template>
        </template>

        <!-- ═══ Tab 3: 档详情 ═══ -->
        <template v-if="activeTab === 'detail'">
          <!-- 未选档提示 -->
          <div v-if="!selectedCampaignId" class="text-center text-ink-soft text-sm py-8">
            请先在「游玩档」tab 点击一个档
          </div>

          <template v-else>
            <!-- 刷新按钮 -->
            <div class="flex justify-end mb-1">
              <button @click="refreshDetail()" class="px-2.5 py-1 rounded-full text-xs font-medium bg-bg text-ink-soft hover:bg-line border border-line">刷新</button>
            </div>

            <!-- 子 Tab 切换条 -->
            <div class="flex gap-1 bg-surface rounded-xl p-1 mb-3">
              <button
                v-for="st in [{key:'instances',label:'角色实例'},{key:'knowledge',label:'知识'},{key:'tasks',label:'任务'},{key:'summaries',label:'摘要'}]"
                :key="st.key"
                @click="detailSubTab = st.key"
                class="flex-1 py-1.5 text-xs font-medium rounded-lg transition-colors"
                :class="detailSubTab === st.key ? 'bg-bg text-accent shadow-sm' : 'text-ink-soft'"
              >{{ st.label }}</button>
            </div>

            <!-- ▸ 子 Tab: 角色实例 -->
            <div v-if="detailSubTab === 'instances'">
              <div v-if="instances.length === 0" class="text-center text-ink-soft text-sm py-8">暂无角色实例</div>

              <div
                v-for="inst in instances" :key="inst.id"
                class="bg-surface rounded-xl border border-line overflow-hidden mb-2"
              >
                <!-- 实例头部 -->
                <div
                  class="flex items-center gap-3 px-3 py-2.5 cursor-pointer hover:bg-bg transition-colors"
                  @click="toggleInstance(inst)"
                >
                  <div class="flex-1 min-w-0">
                    <div class="text-sm font-medium text-ink truncate">{{ inst.name || inst.character_name }}</div>
                    <div class="text-xs text-ink-soft">
                      {{ inst.role_type || '' }}
                      <span v-if="inst.is_active" class="text-ok ml-1">● 存活</span>
                      <span v-else class="text-ink-soft ml-1">○ 离场</span>
                    </div>
                  </div>
                  <span class="text-ink-soft text-xs">{{ expandedInstanceId === inst.id ? '▲' : '▼' }}</span>
                </div>

                <!-- 实例展开：变量编辑 -->
                <div v-if="expandedInstanceId === inst.id" class="border-t border-line px-3 py-2 space-y-2">
                  <div class="text-xs font-medium text-ink-soft mb-1">变量</div>
                  <div v-if="instanceVariables.length === 0" class="text-xs text-ink-soft">暂无变量</div>
                  <div
                    v-for="v in instanceVariables" :key="v.key"
                    class="flex items-center gap-2"
                  >
                    <span class="text-xs text-ink-soft w-24 truncate shrink-0" :title="v.key">{{ v.key }}</span>
                    <input
                      :value="typeof v.value === 'object' ? JSON.stringify(v.value) : String(v.value)"
                      @change="handleVariableChange(inst.id, v.key, $event.target.value)"
                      class="flex-1 px-2 py-1 text-xs rounded border border-line bg-bg focus:outline-none focus:border-accent"
                    />
                  </div>
                </div>
              </div>
            </div>

            <!-- ▸ 子 Tab: 知识 -->
            <div v-if="detailSubTab === 'knowledge'">
              <div v-if="knowledge.length === 0" class="text-center text-ink-soft text-sm py-8">暂无知识</div>

              <template v-else>
                <div v-for="src in ['witnessed','told_by_other','inferred','backstory']" :key="src">
                  <div v-if="knowledge.filter(k => k.source === src).length > 0" class="mb-3">
                    <div class="text-xs font-medium text-ink-soft mb-1.5">{{ knowledgeSourceText(src) }}</div>
                    <div
                      v-for="k in knowledge.filter(k => k.source === src)" :key="k.id"
                      class="bg-surface rounded-xl border border-line px-3 py-2 mb-1.5"
                    >
                      <div class="text-xs text-ink">{{ k.content }}</div>
                      <div v-if="k.character_name" class="text-[10px] text-ink-soft mt-1">— {{ k.character_name }}</div>
                    </div>
                  </div>
                </div>
              </template>
            </div>

            <!-- ▸ 子 Tab: 任务 -->
            <div v-if="detailSubTab === 'tasks'">
              <!-- 新建任务表单 -->
              <div v-if="showNewTask" class="bg-surface rounded-xl border border-line p-3 space-y-2 mb-3">
                <div class="text-xs font-medium text-ink">新建任务</div>
                <input
                  v-model="newTaskTitle"
                  placeholder="任务标题"
                  class="w-full px-3 py-2 text-sm rounded-lg border border-line bg-bg focus:outline-none focus:border-accent"
                  @keyup.enter="handleCreateTask"
                />
                <textarea
                  v-model="newTaskDesc"
                  placeholder="任务描述（可选）"
                  rows="2"
                  class="w-full px-3 py-2 text-sm rounded-lg border border-line bg-bg focus:outline-none focus:border-accent resize-none"
                ></textarea>
                <div class="flex gap-2">
                  <button @click="showNewTask = false" class="flex-1 py-1.5 rounded-lg text-xs bg-bg text-ink-soft">取消</button>
                  <button
                    @click="handleCreateTask"
                    :disabled="!newTaskTitle.trim()"
                    class="flex-1 py-1.5 rounded-lg text-xs font-medium bg-accent text-white disabled:opacity-50"
                  >创建</button>
                </div>
              </div>

              <div v-if="tasks.length === 0 && !showNewTask" class="text-center py-8">
                <div class="text-ink-soft text-sm mb-3">暂无任务</div>
                <button @click="showNewTask = true" class="px-4 py-2 rounded-lg text-xs font-medium bg-accent text-white">新建任务</button>
              </div>

              <!-- 任务列表 -->
              <div
                v-for="task in tasks" :key="task.id"
                class="bg-surface rounded-xl border border-line px-3 py-2.5 mb-2"
              >
                <div class="flex items-start gap-2">
                  <div class="flex-1 min-w-0">
                    <div class="text-sm font-medium text-ink">{{ task.title }}</div>
                    <div v-if="task.description" class="text-xs text-ink-soft mt-0.5 line-clamp-2">{{ task.description }}</div>
                    <div class="flex items-center gap-2 mt-1.5">
                      <span class="px-1.5 py-0.5 rounded text-[10px]" :class="taskStatusClass(task.status)">
                        {{ taskStatusText(task.status) }}
                      </span>
                      <span v-if="task.source_tags?.length" class="text-[10px] text-ink-soft">
                        {{ task.source_tags.join(', ') }}
                      </span>
                    </div>
                  </div>
                  <div class="flex flex-col gap-1 shrink-0">
                    <button
                      v-if="task.status !== 'completed' && task.status?.likely_completed == null"
                      @click="handleCompleteTask(task.id)"
                      class="px-2 py-0.5 rounded-full text-[10px] font-medium bg-ok/10 text-ok hover:bg-ok/20"
                    >完成</button>
                    <button
                      v-if="task.status !== 'completed' && task.status !== 'abandoned'"
                      @click="handleAbandonTask(task.id)"
                      class="px-2 py-0.5 rounded-full text-[10px] font-medium bg-ink-soft/10 text-ink-soft hover:bg-ink-soft/20"
                    >放弃</button>
                  </div>
                </div>
              </div>

              <!-- 有任务时仍可新建 -->
              <button
                v-if="tasks.length > 0 && !showNewTask"
                @click="showNewTask = true"
                class="w-full py-2 rounded-lg text-xs font-medium bg-bg text-ink-soft hover:bg-line border border-dashed border-line"
              >+ 新建任务</button>
            </div>

            <!-- ▸ 子 Tab: 摘要 -->
            <div v-if="detailSubTab === 'summaries'">
              <div v-if="summaries.length === 0" class="text-center text-ink-soft text-sm py-8">暂无摘要</div>

              <div
                v-for="s in summaries" :key="s.id"
                class="bg-surface rounded-xl border border-line px-3 py-2.5 mb-2"
              >
                <div class="flex items-center gap-2 mb-1">
                  <span class="text-xs font-medium text-ink">第 {{ s.round_number }} 轮</span>
                  <span v-if="s.created_at" class="text-[10px] text-ink-soft">{{ s.created_at }}</span>
                </div>
                <div class="text-xs text-ink-soft leading-relaxed">{{ s.summary }}</div>
              </div>
            </div>
          </template>
        </template>
      </div>
    </div>
  </div>
</template>
