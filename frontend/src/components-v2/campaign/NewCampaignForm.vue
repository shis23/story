<script setup>
import { watch } from 'vue'
import { campaignCardOptionSuffix } from '../../utils/campaignCardStatus.js'
import Overlay from '../ui/Overlay.vue'
import Button from '../ui/Button.vue'
import Input from '../ui/Input.vue'
import Select from '../ui/Select.vue'

// v-model:show 控制显隐；form 是 AppV2 持有的 useNewCampaignForm 实例
// （单一状态源：AppV2 的 openNewCampaignDialog 已完成打开初始化，
// 组件内不再自建实例 + watch 重复加载，此前每次打开都会 listCards/getCard 两次）
const props = defineProps({
  show: { type: Boolean, default: false },
  form: { type: Object, required: true },
})
const emit = defineEmits(['update:show', 'close', 'created'])

const {
  newCampaignCards,
  newCampaignCardId,
  newCampaignName,
  newCampaignGreetingIndex,
  creatingCampaign,
  newCampaignGreetingOptions,
  loadNewCampaignCardDetail,
  handleCreateCampaign,
} = props.form

// 卡片选项(带识别状态后缀)
const cardOptions = (cards) =>
  cards.map((c) => ({ value: c.id, label: `${c.name}${campaignCardOptionSuffix(c)}` }))

// greeting 选项(用 index 作 value)
const greetingOptions = (opts) => opts.map((o, i) => ({ value: i, label: o.label }))

// ─── 选卡变更加载详情 ───
watch(newCampaignCardId, async () => {
  await loadNewCampaignCardDetail()
})

async function onCreate() {
  // 仅创建成功才关闭：失败时弹层保持打开，保住用户已填内容
  const ok = await handleCreateCampaign()
  if (ok) {
    emit('created')
    emit('update:show', false)
  }
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
