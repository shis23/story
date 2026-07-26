<script setup>
// L7-A 重型卡内嵌可见挂载（方案 A：消息区内嵌折叠面板）。
// - 默认收起：只渲染 chip 行，不挂载任何 iframe（重脚本零执行）
// - 展开即挂载：点击 chip = 用户手势（配合 iframe allow=autoplay 解锁播放）
// - 一次一个活跃：展开新 chip 自动卸载上一个（key 换 label 强制重建）
// 入口数据：卡 manifest 拆出的重型 TH 壳清单（utils/heavyShellApps.js）。
import { computed, ref, watch } from 'vue'
import TavernHelperRuntime from '../../components/TavernHelperRuntime.vue'
import { heavyShellSizeLabel } from '../../utils/heavyShellApps.js'

const props = defineProps({
  /** 重型 TH 壳（raw manifest shell 对象） */
  shells: { type: Array, default: () => [] },
  /** deferred inline JS 拉取用 */
  characterId: { type: String, default: null },
})
const emit = defineEmits(['var-write'])

const activeLabel = ref('')

const chips = computed(() =>
  props.shells.map((shell, index) => ({
    label: shell.label || `app#${index}`,
    size: heavyShellSizeLabel(shell),
    shell,
  })),
)

const activeChip = computed(
  () => chips.value.find((c) => c.label === activeLabel.value) || null,
)

function toggle(label) {
  activeLabel.value = activeLabel.value === label ? '' : label
}

// 换卡/换 Campaign（清单变化）后收起，避免旧应用挂在新上下文里
watch(
  () => props.shells,
  () => {
    activeLabel.value = ''
  },
)
</script>

<template>
  <div v-if="chips.length" class="heavy-shell-dock space-y-1.5">
    <div class="flex flex-wrap items-center gap-1.5">
      <span class="text-[11px] text-ink-faint">卡片应用</span>
      <button
        v-for="chip in chips"
        :key="chip.label"
        type="button"
        class="min-h-6 px-2 rounded-full border text-[11px] transition-colors"
        :class="
          chip.label === activeLabel
            ? 'border-accent text-accent bg-accent/5'
            : 'border-line text-ink-soft hover:border-accent-border hover:text-ink'
        "
        @click="toggle(chip.label)"
      >
        {{ chip.label }}<span v-if="chip.size" class="text-ink-faint"> · {{ chip.size }}</span>
        <span class="ml-0.5">{{ chip.label === activeLabel ? '▾' : '▸' }}</span>
      </button>
    </div>
    <!-- 展开才挂载；key 绑 label：切换 chip 必定卸载旧 iframe 再建新的 -->
    <div
      v-if="activeChip"
      :key="activeChip.label"
      class="rounded-lg border border-line bg-surface-2/30 p-1.5"
    >
      <TavernHelperRuntime
        :shells="[activeChip.shell]"
        :character-id="characterId"
        :visible="true"
        :show-status="true"
        :auto-run="true"
        placement="dock"
        @var-write="emit('var-write', $event)"
      />
    </div>
  </div>
</template>
