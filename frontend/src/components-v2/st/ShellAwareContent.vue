<script setup>
/**
 * ShellAwareContent — display_content 分流：
 *   - 含 `$('body').load(url)` 的可执行壳 → CardShellHost（宿主代持，不降级）
 *   - 剩余静态片段 → RichContent
 *
 * design 层仍可只注入 contentComponent=本组件；不破坏 MessageItem 契约。
 */
import { computed } from 'vue'
import RichContent from './RichContent.vue'
import CardShellHost from '../../components/CardShellHost.vue'
import { extractShellMountsFromDisplay } from '../../utils/cardShellDisplay.js'
import { useCampaignStore } from '../../stores/campaign.js'

const props = defineProps({
  content: { type: String, default: '' },
  sourceContent: { type: String, default: '' },
})

const parsed = computed(() => extractShellMountsFromDisplay(props.content))
const mounts = computed(() => parsed.value.mounts)
const residual = computed(() => parsed.value.residualText)
const hasShell = computed(() => mounts.value.length > 0)
// Message-local shells are rendered outside AppV2's #shell slot. Bind them
// explicitly to the active Campaign so their TavernHelper worldbook APIs use
// the same persisted truth source as the visible status/opening shells.
const campaign = useCampaignStore()
const activeCampaignId = computed(() => campaign.activeCampaign?.id || null)

function heightFor(kind) {
  if (kind === 'status') return '110px'
  if (kind === 'opening_home' || kind === 'opening_custom') return '420px'
  return '280px'
}

function compactFor(kind) {
  return kind === 'status'
}

function labelFor(kind) {
  if (kind === 'status') return '消息状态壳'
  if (kind === 'opening_home') return '消息首页壳'
  if (kind === 'opening_custom') return '消息自定义开局壳'
  return '消息 HTML 壳'
}
</script>

<template>
  <div class="shell-aware-content space-y-2">
    <template v-if="hasShell">
      <CardShellHost
        v-for="(m, i) in mounts"
        :key="m.url + ':' + i"
        :url="m.url"
        :campaign-id="activeCampaignId"
        :label="labelFor(m.kind)"
        :compact="compactFor(m.kind)"
        :height="heightFor(m.kind)"
      />
      <RichContent
        v-if="residual"
        :content="residual"
        :source-content="sourceContent"
      />
    </template>
    <RichContent
      v-else
      :content="content"
      :source-content="sourceContent"
    />
  </div>
</template>
