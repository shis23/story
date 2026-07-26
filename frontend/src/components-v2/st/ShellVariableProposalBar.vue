<script setup>
/**
 * 卡壳变量写入确认条（写作面 after-messages 槽，MvuStatusPanel 上方）
 *
 * 卡 JS 经 var_write 请求写一等 Campaign/实例变量时不再直写，先在此列出
 * 待确认提案；用户逐条或全部 应用/拒绝。纯 props 渲染，持久化由 AppV2 承担。
 */
import { previewShellVariableValue } from '../../utils/shellVariableProposals.js'

defineProps({
  /** [{ id, key, value, source }] */
  proposals: { type: Array, default: () => [] },
  /** 应用中：按钮禁用 */
  busy: { type: Boolean, default: false },
})

const emit = defineEmits(['apply', 'reject', 'apply-all', 'reject-all'])
</script>

<template>
  <div
    v-if="proposals.length"
    class="rounded-lg border border-warn/40 bg-warn/5 px-2.5 py-2 space-y-1.5 text-xs"
    data-testid="shell-var-proposal-bar"
  >
    <div class="flex items-center gap-2">
      <span class="text-ink font-medium">卡片请求写入 {{ proposals.length }} 个变量</span>
      <span class="flex-1" />
      <button
        type="button"
        class="px-1.5 py-0.5 rounded border border-line text-ink-soft hover:text-ink disabled:opacity-50"
        :disabled="busy"
        data-testid="shell-var-apply-all"
        @click="emit('apply-all')"
      >
        全部应用
      </button>
      <button
        type="button"
        class="px-1.5 py-0.5 rounded border border-line text-ink-soft hover:text-ink disabled:opacity-50"
        :disabled="busy"
        data-testid="shell-var-reject-all"
        @click="emit('reject-all')"
      >
        全部拒绝
      </button>
    </div>
    <div
      v-for="p in proposals"
      :key="p.id"
      class="flex items-center gap-2"
      data-testid="shell-var-proposal-row"
    >
      <code class="text-ink truncate max-w-[40%]">{{ p.key }}</code>
      <span class="text-ink-soft truncate flex-1">← {{ previewShellVariableValue(p.value) }}</span>
      <button
        type="button"
        class="px-1.5 py-0.5 rounded border border-line text-ink-soft hover:text-ink disabled:opacity-50"
        :disabled="busy"
        :data-testid="`shell-var-apply-${p.id}`"
        @click="emit('apply', p.id)"
      >
        应用
      </button>
      <button
        type="button"
        class="px-1.5 py-0.5 rounded border border-line text-ink-soft hover:text-ink disabled:opacity-50"
        :disabled="busy"
        :data-testid="`shell-var-reject-${p.id}`"
        @click="emit('reject', p.id)"
      >
        拒绝
      </button>
    </div>
  </div>
</template>
