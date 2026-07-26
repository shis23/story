<script setup>
import { watch } from 'vue'
import { useNewCampaignForm } from '../../composables/useNewCampaignForm.js'
import { campaignCardOptionSuffix } from '../../utils/campaignCardStatus.js'
import Overlay from '../ui/Overlay.vue'
import Button from '../ui/Button.vue'
import Input from '../ui/Input.vue'
import Select from '../ui/Select.vue'

// v-model:show 控制显隐
const props = defineProps({
  show: { type: Boolean, default: false },
  // 注入给 composable 的回调(可选)
  loadInstanceNameMap: { type: Function, default: () => {} },
  applyConversation: { type: Function, default: () => {} },
  broadcastPluginEvent: { type: Function, default: () => {} },
  loadConversationHistory: { type: Function, default: () => {} },
  refreshCardShellManifest: { type: Function, default: () => {} },
  openingShellStarted: { type: Function, default: () => {} },
  alertDialog: { type: Function, default: null },
})
const emit = defineEmits(['update:show', 'close', 'created'])

// ─── 消费 composable ───
const form = useNewCampaignForm({
  loadInstanceNameMap: props.loadInstanceNameMap,
  applyConversation: props.applyConversation,
  broadcastPluginEvent: props.broadcastPluginEvent,
  loadConversationHistory: props.loadConversationHistory,
  refreshCardShellManifest: props.refreshCardShellManifest,
  openingShellStarted: props.openingShellStarted,
  alertDialog: props.alertDialog,
})

const {
  newCampaignCards,
  newCampaignCardId,
  newCampaignName,
  newCampaignGreetingIndex,
  creatingCampaign,
  newCampaignGreetingOptions,
  loadNewCampaignCardDetail,
  openNewCampaignDialog,
  handleCreateCampaign,
} = form

// 卡片选项(带识别状态后缀)
const cardOptions = (cards) =>
  cards.map((c) => ({ value: c.id, label: `${c.name}${campaignCardOptionSuffix(c)}` }))

// greeting 选项(用 index 作 value)
const greetingOptions = (opts) => opts.map((o, i) => ({ value: i, label: o.label }))

// ─── 弹层打开时初始化 ───
watch(
  () => props.show,
  async (val) => {
    if (val) await openNewCampaignDialog()
  },
)

// ─── 选卡变更加载详情 ───
watch(newCampaignCardId, async () => {
  await loadNewCampaignCardDetail()
})

async function onCreate() {
  await handleCreateCampaign()
  // 创建成功后表单内部已关 showNewCampaignForm,这里同步外部 show
  emit('created')
  emit('update:show', false)
}

function onClose() {
  emit('update:show', false)
  emit('close')
}
</script>

<template>
  <Overlay :show="show" side="center" title="新建 Campaign" @update:show="onClose">
    <div class="p-4 space-y-4">
      <div v-if="newCampaignCards.length === 0" class="text-center text-ink-soft text-sm py-6">
        还没有已导入的角色卡<br>
        <span class="text-xs text-ink-faint">先点「导入」添加角色卡并识别角色</span>
      </div>
      <template v-else>
        <div>
          <label class="text-xs text-ink-soft mb-1.5 block">选择角色卡</label>
          <Select
            v-model="newCampaignCardId"
            :options="cardOptions(newCampaignCards)"
            placeholder="选择角色卡"
          />
        </div>
        <div v-if="newCampaignGreetingOptions.length > 1">
          <label class="text-xs text-ink-soft mb-1.5 block">开场白</label>
          <Select
            v-model="newCampaignGreetingIndex"
            :options="greetingOptions(newCampaignGreetingOptions)"
          />
        </div>
        <div>
          <label class="text-xs text-ink-soft mb-1.5 block">Campaign 名称</label>
          <Input v-model="newCampaignName" placeholder="如：第一周目" @keyup.enter="onCreate" />
        </div>
        <div class="flex gap-2 pt-1">
          <Button variant="default" size="md" class="flex-1" @click="onClose">取消</Button>
          <Button
            variant="primary"
            size="md"
            class="flex-1"
            :disabled="!newCampaignCardId || !newCampaignName.trim() || creatingCampaign"
            :loading="creatingCampaign"
            @click="onCreate"
          >{{ creatingCampaign ? '创建中…' : '创建并开始' }}</Button>
        </div>
      </template>
    </div>
  </Overlay>
</template>
