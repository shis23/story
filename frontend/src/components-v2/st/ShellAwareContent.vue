<script setup>
/**
 * ShellAwareContent — display_content 分流：
 *   - 含 `$('body').load(url)` 的可执行壳 → CardShellHost（宿主代持，不降级）
 *   - 剩余静态片段 → RichContent
 *
 * H3 挂壳分级：壳文档拿到完整 bridge 权限，挂载即代码执行。只有卡 manifest
 * 注册过的 URL（导入时抽取）自动挂载；消息内出现的其它 .load URL 先渲染
 * 确认卡，用户点「信任并挂载本次」才挂——阻断「卡正则把输出改写成
 * .load(攻击者 URL)」的远程代码链。
 *
 * design 层仍可只注入 contentComponent=本组件；不破坏 MessageItem 契约。
 */
import { computed, inject, ref } from 'vue'
import RichContent from './RichContent.vue'
import CardShellHost from '../../components/CardShellHost.vue'
import {
  extractInlineShellDocsFromDisplay,
  extractShellMountsFromDisplay,
  matchesAnyInlineShellTrigger,
  partitionShellMountsByTrust,
} from '../../utils/cardShellDisplay.js'
import { useCampaignStore } from '../../stores/campaign.js'

const props = defineProps({
  content: { type: String, default: '' },
  sourceContent: { type: String, default: '' },
})

const parsed = computed(() => extractShellMountsFromDisplay(props.content))
const mounts = computed(() => parsed.value.mounts)
// H4：.load 之外，display 里可执行的内联 HTML 文档（卡正则的巨型 replace
// 输出）也走 CardShellHost 沙箱，而不是被 RichContent/DOMPurify 剥壳。
const inlineParsed = computed(() => extractInlineShellDocsFromDisplay(parsed.value.residualText))
const inlineDocs = computed(() => inlineParsed.value.docs)
const residual = computed(() => inlineParsed.value.residualText)
const messageShellLayout = inject('storyforgeCardShellLayout', null)

function unwrapLayoutList(source) {
  const value = source && typeof source === 'object' && 'value' in source
    ? source.value
    : source
  return Array.isArray(value) ? value : []
}

const suppressedMessageShellUrls = computed(() =>
  unwrapLayoutList(messageShellLayout?.suppressedMessageShellUrls),
)
const trustedMessageShellUrls = computed(() =>
  unwrapLayoutList(messageShellLayout?.trustedMessageShellUrls),
)
const inlineShellTriggers = computed(() =>
  unwrapLayoutList(messageShellLayout?.inlineShellTriggers),
)
const renderedMounts = computed(() =>
  mounts.value.filter((mount) => !suppressedMessageShellUrls.value.includes(mount.url)),
)
// 本会话内用户显式放行的 URL（不落盘：重启后重新确认）
const approvedShellUrls = ref([])
const mountPartition = computed(() => partitionShellMountsByTrust(
  renderedMounts.value,
  trustedMessageShellUrls.value,
  approvedShellUrls.value,
))
const allowedMounts = computed(() => mountPartition.value.allowed)
const pendingMounts = computed(() => mountPartition.value.needsConfirmation)

// 内联文档的信任锚：消息源文命中卡 manifest 里 InlineHtml 壳的 find_regex
// （即卡自己的正则真的会改写这条消息），否则逐条确认。
const inlineDocsTrusted = computed(() => matchesAnyInlineShellTrigger(
  props.sourceContent || props.content,
  inlineShellTriggers.value,
))
const approvedInlineDocs = ref([])
const allowedInlineDocs = computed(() => (
  inlineDocsTrusted.value
    ? inlineDocs.value
    : inlineDocs.value.filter((_, i) => approvedInlineDocs.value.includes(i))
))
const pendingInlineDocs = computed(() => (
  inlineDocsTrusted.value
    ? []
    : inlineDocs.value
      .map((doc, i) => ({ doc, index: i }))
      .filter(({ index }) => !approvedInlineDocs.value.includes(index))
))
const hasParsedShell = computed(() => mounts.value.length > 0 || inlineDocs.value.length > 0)
const hasShell = computed(() => allowedMounts.value.length > 0)

function approveShellUrl(url) {
  if (!url || approvedShellUrls.value.includes(url)) return
  approvedShellUrls.value = [...approvedShellUrls.value, url]
}

function approveInlineDoc(index) {
  if (approvedInlineDocs.value.includes(index)) return
  approvedInlineDocs.value = [...approvedInlineDocs.value, index]
}

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
        v-for="(m, i) in allowedMounts"
        :key="m.url + ':' + i"
        :url="m.url"
        :campaign-id="activeCampaignId"
        :label="labelFor(m.kind)"
        :compact="compactFor(m.kind)"
        :height="heightFor(m.kind)"
      />
    </template>
    <CardShellHost
      v-for="(doc, i) in allowedInlineDocs"
      :key="'inline:' + i"
      :html="doc.html"
      :campaign-id="activeCampaignId"
      :label="labelFor(doc.kind)"
      :auto-height="true"
      :height="heightFor(doc.kind)"
    />
    <div
      v-for="{ index } in pendingInlineDocs"
      :key="'pending-inline:' + index"
      class="rounded-lg border border-warn/40 bg-warn/5 px-3 py-2 text-xs space-y-1"
      data-testid="shell-inline-confirm"
    >
      <div class="text-ink">消息包含可执行的内联界面，但未命中本卡的正则触发器：</div>
      <button
        type="button"
        class="px-2 py-0.5 rounded border border-line text-ink-soft hover:text-ink"
        data-testid="shell-inline-approve"
        @click="approveInlineDoc(index)"
      >
        信任并挂载本次
      </button>
    </div>
    <div
      v-for="m in pendingMounts"
      :key="'pending:' + m.url"
      class="rounded-lg border border-warn/40 bg-warn/5 px-3 py-2 text-xs space-y-1"
      data-testid="shell-mount-confirm"
    >
      <div class="text-ink">消息请求挂载未注册的壳页面（不在本卡清单内）：</div>
      <code class="block truncate text-ink-soft">{{ m.url }}</code>
      <button
        type="button"
        class="px-2 py-0.5 rounded border border-line text-ink-soft hover:text-ink"
        data-testid="shell-mount-approve"
        @click="approveShellUrl(m.url)"
      >
        信任并挂载本次
      </button>
    </div>
    <RichContent
      v-if="residual || !hasParsedShell"
      :content="hasParsedShell ? residual : content"
      :source-content="sourceContent"
    />
  </div>
</template>
