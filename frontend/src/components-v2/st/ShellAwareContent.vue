<script setup>
/**
 * ShellAwareContent — display_content 分段渲染（文本与壳原地交错）：
 *   - `$('body').load(url)` 型壳 → CardShellHost :url（宿主代持，不降级）
 *   - display 里含 script 的内联 HTML 文档 → CardShellHost :html（H4）
 *   - 其余叙事文本 → RichContent，按原文顺序逐段渲染
 *
 * 顺序保真：酒馆助手在消息 DOM 流里原地挂 iframe，卡按「UI 出现在叙事
 * 中间」设计。segmentShellContent 单遍认领 span，壳渲染在它在原文中的
 * 位置，不再吊顶。
 *
 * H3/H4 信任分级（逻辑不变，只换渲染位置）：
 *   - .load 仅卡 manifest 注册 URL 自动挂载，其余原地确认卡；
 *   - 内联文档需消息源文命中卡 InlineHtml 壳的 find_regex，否则原地确认。
 * 确认门同时是天然的懒加载闸门（重内联壳未放行不占资源）——将来若做
 * 「记住信任」，默认仍必须逐次确认，不得改成全自动挂载。
 *
 * suppress 集合按 URL 匹配，只作用于 .load 壳（开场/状态壳转专属槽位）；
 * 内联文档没有 URL，不参与 suppress——不要给它接这个语义。
 *
 * design 层仍可只注入 contentComponent=本组件；不破坏 MessageItem 契约。
 */
import { computed, inject, ref } from 'vue'
import RichContent from './RichContent.vue'
import CardShellHost from '../../components/CardShellHost.vue'
import {
  matchesAnyInlineShellTrigger,
  segmentShellContent,
} from '../../utils/cardShellDisplay.js'
import { useCampaignStore } from '../../stores/campaign.js'

const props = defineProps({
  content: { type: String, default: '' },
  sourceContent: { type: String, default: '' },
})

const segments = computed(() => segmentShellContent(props.content))
const messageShellLayout = inject('storyforgeCardShellLayout', null)

function unwrapLayoutList(source) {
  const value = source && typeof source === 'object' && 'value' in source
    ? source.value
    : source
  return Array.isArray(value) ? value : []
}

const suppressedSet = computed(() =>
  new Set(unwrapLayoutList(messageShellLayout?.suppressedMessageShellUrls)),
)
const trustedSet = computed(() =>
  new Set(unwrapLayoutList(messageShellLayout?.trustedMessageShellUrls)),
)
const inlineShellTriggers = computed(() =>
  unwrapLayoutList(messageShellLayout?.inlineShellTriggers),
)

// 本会话内用户显式放行的壳（不落盘：重启后重新确认）。
// load 壳按 URL 记；内联壳按原文 start 偏移记——流式是尾部追加，已出现
// 段的偏移稳定；流结束的全文改写允许其重挂一次（见 segmentShellContent 注释）。
const approvedShellUrls = ref([])
const approvedInlineStarts = ref([])

const inlineDocsTrusted = computed(() => matchesAnyInlineShellTrigger(
  props.sourceContent || props.content,
  inlineShellTriggers.value,
))

const renderSegments = computed(() => {
  const out = []
  segments.value.forEach((seg, index) => {
    if (seg.type === 'text') {
      out.push({ kind: 'text', key: `text:${index}`, content: seg.content })
      return
    }
    if (seg.mode === 'load') {
      if (suppressedSet.value.has(seg.url)) return
      const ok = trustedSet.value.has(seg.url)
        || approvedShellUrls.value.includes(seg.url)
      out.push(ok
        ? { kind: 'host-url', key: `load:${seg.url}`, url: seg.url, shellKind: seg.kind }
        : { kind: 'confirm-url', key: `confirm:${seg.url}`, url: seg.url })
      return
    }
    const ok = inlineDocsTrusted.value
      || approvedInlineStarts.value.includes(seg.start)
    out.push(ok
      ? { kind: 'host-html', key: `inline:${seg.start}`, html: seg.html, shellKind: seg.kind }
      : { kind: 'confirm-inline', key: `confirm-inline:${seg.start}`, start: seg.start })
  })
  return out
})

function approveShellUrl(url) {
  if (!url || approvedShellUrls.value.includes(url)) return
  approvedShellUrls.value = [...approvedShellUrls.value, url]
}

function approveInlineDoc(start) {
  if (approvedInlineStarts.value.includes(start)) return
  approvedInlineStarts.value = [...approvedInlineStarts.value, start]
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
    <template v-for="seg in renderSegments" :key="seg.key">
      <RichContent
        v-if="seg.kind === 'text'"
        :content="seg.content"
        :source-content="sourceContent"
      />
      <CardShellHost
        v-else-if="seg.kind === 'host-url'"
        :url="seg.url"
        :campaign-id="activeCampaignId"
        :label="labelFor(seg.shellKind)"
        :compact="compactFor(seg.shellKind)"
        :height="heightFor(seg.shellKind)"
      />
      <CardShellHost
        v-else-if="seg.kind === 'host-html'"
        :html="seg.html"
        :campaign-id="activeCampaignId"
        :label="labelFor(seg.shellKind)"
        :auto-height="true"
        :height="heightFor(seg.shellKind)"
      />
      <div
        v-else-if="seg.kind === 'confirm-url'"
        class="rounded-lg border border-warn/40 bg-warn/5 px-3 py-2 text-xs space-y-1"
        data-testid="shell-mount-confirm"
      >
        <div class="text-ink">消息请求挂载未注册的壳页面（不在本卡清单内）：</div>
        <code class="block truncate text-ink-soft">{{ seg.url }}</code>
        <button
          type="button"
          class="px-2 py-0.5 rounded border border-line text-ink-soft hover:text-ink"
          data-testid="shell-mount-approve"
          @click="approveShellUrl(seg.url)"
        >
          信任并挂载本次
        </button>
      </div>
      <div
        v-else
        class="rounded-lg border border-warn/40 bg-warn/5 px-3 py-2 text-xs space-y-1"
        data-testid="shell-inline-confirm"
      >
        <div class="text-ink">消息包含可执行的内联界面，但未命中本卡的正则触发器：</div>
        <button
          type="button"
          class="px-2 py-0.5 rounded border border-line text-ink-soft hover:text-ink"
          data-testid="shell-inline-approve"
          @click="approveInlineDoc(seg.start)"
        >
          信任并挂载本次
        </button>
      </div>
    </template>
  </div>
</template>
