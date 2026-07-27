<script setup>
import { computed, ref } from 'vue'
import BaseOverlay from '../../components/base/BaseOverlay.vue'
import Badge from '../ui/Badge.vue'
import Button from '../ui/Button.vue'

const props = defineProps({
  character: { type: Object, required: true },
})

const emit = defineEmits(['close', 'write'])
const activeSection = ref('overview')

const sourceCard = computed(() => props.character?._card || null)
const definitions = computed(() => sourceCard.value?.character_definitions || [])
const worldEntries = computed(() => props.character?.world_info_entries || [])
const sourceFields = computed(() => [
  { key: 'description', label: '角色描述', value: props.character?.description },
  { key: 'personality', label: '性格原文', value: props.character?.personality },
  { key: 'scenario', label: '场景设定', value: props.character?.scenario },
  { key: 'first_mes', label: '开场白', value: props.character?.first_mes },
  { key: 'system_prompt', label: '系统提示词', value: props.character?.system_prompt, mono: true },
].filter((field) => String(field.value || '').trim()))

const sections = computed(() => [
  { key: 'overview', label: '概览', count: definitions.value.length },
  { key: 'source', label: '原始资料', count: sourceFields.value.length },
  { key: 'world', label: '世界书', count: worldEntries.value.length },
])

function roleVariant(roleType) {
  const normalized = String(roleType || '').toLowerCase()
  if (normalized === 'protagonist') return 'accent'
  if (normalized === 'supporting') return 'ok'
  return 'neutral'
}
</script>

<template>
  <BaseOverlay
    :model-value="true"
    title="角色详情"
    size="lg"
    position="center"
    surface="surface"
    fixed-height
    @close="emit('close')"
  >
    <div class="relative overflow-hidden border-b border-line bg-bg">
      <div class="absolute -right-16 -top-24 h-56 w-56 rounded-full bg-accent-soft blur-3xl pointer-events-none"></div>
      <div class="relative p-5 sm:p-6">
        <div class="mb-4 flex items-center justify-between gap-3">
          <span class="text-[10px] font-semibold uppercase tracking-[0.2em] text-ink-faint">
            Character card<span v-if="character.spec_version"> · ST {{ character.spec_version }}</span>
          </span>
          <Button variant="default" size="sm" @click="emit('write')">
            进入写作
            <span aria-hidden="true">→</span>
          </Button>
        </div>

        <div class="flex items-center gap-4">
          <div class="flex h-16 w-16 shrink-0 items-center justify-center rounded-2xl border border-accent-border bg-accent-soft text-2xl font-semibold text-accent-bright shadow-card">
            {{ character.name?.charAt(0) || '?' }}
          </div>
          <div class="min-w-0 flex-1">
            <h2 class="truncate text-2xl font-bold tracking-tight text-ink sm:text-3xl">{{ character.name }}</h2>
            <div class="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-ink-soft">
              <span><strong class="font-semibold text-ink">{{ definitions.length }}</strong> 个角色定义</span>
              <span class="text-ink-faint">·</span>
              <span><strong class="font-semibold text-ink">{{ worldEntries.length }}</strong> 条世界书</span>
              <template v-if="character.creator">
                <span class="text-ink-faint">·</span>
                <span class="truncate">{{ character.creator }}</span>
              </template>
            </div>
          </div>
        </div>

        <div v-if="character.tags?.length" class="mt-4 flex flex-wrap gap-1.5">
          <Badge v-for="tag in character.tags" :key="tag" variant="neutral" size="sm">
            {{ tag }}
          </Badge>
        </div>
      </div>
    </div>

    <nav class="sticky top-0 z-10 flex border-b border-line bg-surface px-4 sm:px-6" aria-label="角色详情分区">
      <button
        v-for="section in sections"
        :key="section.key"
        type="button"
        class="relative min-h-12 px-3 text-sm transition-colors"
        :class="activeSection === section.key ? 'font-medium text-ink' : 'text-ink-soft hover:text-ink'"
        :aria-selected="activeSection === section.key"
        @click="activeSection = section.key"
      >
        {{ section.label }}
        <span class="ml-1 text-[10px] text-ink-faint">{{ section.count }}</span>
        <span
          v-if="activeSection === section.key"
          class="absolute inset-x-3 bottom-0 h-0.5 rounded-full bg-accent"
          aria-hidden="true"
        ></span>
      </button>
    </nav>

    <div class="p-5 sm:p-6">
      <section v-if="activeSection === 'overview'" class="space-y-6">
        <div class="flex items-end justify-between gap-4 border-b border-line pb-3">
          <div>
            <div class="text-[10px] font-semibold uppercase tracking-[0.18em] text-accent">Recognition</div>
            <h3 class="mt-1 text-lg font-semibold text-ink">识别出的角色定义</h3>
          </div>
          <span class="text-xs text-ink-soft">{{ definitions.length }} 个</span>
        </div>

        <div v-if="definitions.length" class="divide-y divide-line">
          <article
            v-for="(definition, index) in definitions"
            :key="definition.id || definition.name"
            class="grid gap-4 py-5 first:pt-0 sm:grid-cols-[7.5rem_minmax(0,1fr)]"
          >
            <div>
              <div class="flex items-center gap-2">
                <span class="flex h-7 w-7 items-center justify-center rounded-full bg-accent-soft text-xs font-semibold text-accent-bright">
                  {{ index + 1 }}
                </span>
                <span class="font-semibold text-ink break-words">{{ definition.name }}</span>
              </div>
              <div class="mt-2 flex flex-wrap gap-1.5 sm:pl-9">
                <Badge v-if="definition.role_type" :variant="roleVariant(definition.role_type)" size="sm">
                  {{ definition.role_type }}
                </Badge>
                <Badge v-if="definition.group" variant="neutral" size="sm">{{ definition.group }}</Badge>
              </div>
            </div>

            <div class="min-w-0 space-y-4 border-l-2 border-accent-border pl-4">
              <div v-if="definition.persona_prompt">
                <div class="mb-1 text-[11px] font-semibold uppercase tracking-wider text-ink-faint">人设</div>
                <p class="max-w-[68ch] text-sm leading-7 text-ink whitespace-pre-wrap">{{ definition.persona_prompt }}</p>
              </div>
              <div v-if="definition.behavior_rules">
                <div class="mb-1 text-[11px] font-semibold uppercase tracking-wider text-ink-faint">行为规则</div>
                <p class="max-w-[68ch] text-sm leading-7 text-ink whitespace-pre-wrap">{{ definition.behavior_rules }}</p>
              </div>
              <div v-if="definition.base_backstory?.length">
                <div class="mb-1.5 text-[11px] font-semibold uppercase tracking-wider text-ink-faint">已知背景</div>
                <ul class="max-w-[68ch] space-y-1.5 text-sm leading-6 text-ink-soft">
                  <li v-for="item in definition.base_backstory" :key="item" class="flex gap-2">
                    <span class="mt-2 h-1 w-1 shrink-0 rounded-full bg-accent" aria-hidden="true"></span>
                    <span>{{ item }}</span>
                  </li>
                </ul>
              </div>
            </div>
          </article>
        </div>

        <div v-else class="rounded-xl border border-dashed border-warn/40 bg-warn/10 px-4 py-6 text-center">
          <div class="text-sm font-medium text-warn">还没有识别出的角色定义</div>
          <div class="mt-1 text-xs text-ink-soft">可以在 Campaign 管理的角色卡库中重新识别。</div>
        </div>
      </section>

      <section v-else-if="activeSection === 'source'" class="space-y-4">
        <div class="border-b border-line pb-3">
          <div class="text-[10px] font-semibold uppercase tracking-[0.18em] text-accent">Source material</div>
          <h3 class="mt-1 text-lg font-semibold text-ink">原始卡资料</h3>
          <p class="mt-1 text-xs leading-5 text-ink-soft">长文本默认收起，按需展开，避免遮住角色识别结果。</p>
        </div>

        <div v-if="sourceFields.length" class="space-y-2">
          <details
            v-for="field in sourceFields"
            :key="field.key"
            class="group rounded-xl border border-line bg-bg open:border-accent-border open:shadow-card"
          >
            <summary class="flex min-h-12 cursor-pointer list-none items-center justify-between gap-3 px-4 text-sm font-medium text-ink">
              <span>{{ field.label }}</span>
              <span class="text-ink-faint transition-transform group-open:rotate-90" aria-hidden="true">›</span>
            </summary>
            <div class="border-t border-line px-4 py-4">
              <pre
                class="max-w-[72ch] text-sm leading-7 text-ink-soft whitespace-pre-wrap break-words"
                :class="field.mono ? 'font-mono text-xs' : 'font-sans'"
              >{{ field.value }}</pre>
            </div>
          </details>
        </div>

        <div v-else class="rounded-xl border border-dashed border-line px-4 py-8 text-center text-sm text-ink-soft">
          这张卡没有可展示的原始文本字段。
        </div>
      </section>

      <section v-else class="space-y-4">
        <div class="border-b border-line pb-3">
          <div class="text-[10px] font-semibold uppercase tracking-[0.18em] text-accent">World book</div>
          <h3 class="mt-1 text-lg font-semibold text-ink">世界书</h3>
          <p class="mt-1 text-xs leading-5 text-ink-soft">共 {{ worldEntries.length }} 条，点击关键词查看完整内容。</p>
        </div>

        <div v-if="worldEntries.length" class="space-y-2">
          <details
            v-for="(entry, index) in worldEntries"
            :key="index"
            class="group rounded-xl border border-line bg-bg open:border-accent-border open:shadow-card"
          >
            <summary class="flex min-h-12 cursor-pointer list-none items-center gap-3 px-4">
              <span class="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-surface-2 text-[10px] font-medium text-ink-soft">
                {{ index + 1 }}
              </span>
              <span class="min-w-0 flex-1 truncate text-sm font-medium text-ink">
                {{ entry.keys?.join(', ') || `条目 ${index + 1}` }}
              </span>
              <span class="text-ink-faint transition-transform group-open:rotate-90" aria-hidden="true">›</span>
            </summary>
            <p class="max-w-[72ch] border-t border-line px-4 py-4 text-sm leading-7 text-ink-soft whitespace-pre-wrap">
              {{ entry.content }}
            </p>
          </details>
        </div>

        <div v-else class="rounded-xl border border-dashed border-line px-4 py-8 text-center text-sm text-ink-soft">
          这张卡没有世界书条目。
        </div>
      </section>
    </div>
  </BaseOverlay>
</template>
