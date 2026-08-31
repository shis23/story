<script setup>
import { ref } from 'vue'
import { metaExplainGeneration } from '../../tauri-api.js'
import { explainGenerationFlow } from '../../utils/metaPanelFlow.js'
import Button from '../ui/Button.vue'
import Badge from '../ui/Badge.vue'
import CodeBlock from '../ui/CodeBlock.vue'
import EmptyState from '../ui/EmptyState.vue'
import LoadingState from '../ui/LoadingState.vue'
import { errorText } from '../../utils/errorText.js'

// 生成溯源：读 lastConversationNode，调 meta_explain_generation 展示 trace/provenance。
// props.lastConversationNode = { conversation_id, node_id }（可能为 null）

const props = defineProps({
  lastConversationNode: { type: Object, default: null },
})

const emit = defineEmits(['error'])

// ─── 状态 ───
const loading = ref(false)
const result = ref(null) // GenerationExplanation
const error = ref('')

async function handleExplain() {
  if (!props.lastConversationNode) return
  loading.value = true
  result.value = null
  error.value = ''
  try {
    result.value = await explainGenerationFlow({
      lastConversationNode: props.lastConversationNode,
      explainGeneration: metaExplainGeneration,
    })
  } catch (e) {
    error.value = '生成溯源失败: ' + errorText(e)
    emit('error', error.value)
  } finally {
    loading.value = false
  }
}
</script>

<template>
  <div class="space-y-3">
    <!-- 入口说明 -->
    <div class="bg-surface rounded-lg border border-line p-3 text-xs text-ink-soft">
      读取上一条助手消息的生成溯源：场景、子 Agent、Seed 等。
    </div>

    <!-- 无节点：提示 -->
    <EmptyState
      v-if="!lastConversationNode"
      title="暂无可用对话节点"
      description="发送一条消息后再查看溯源"
    />

    <template v-else>
      <Button
        variant="primary"
        size="md"
        :loading="loading"
        :disabled="loading"
        @click="handleExplain"
      >{{ loading ? '查询中…' : '解释上一条生成' }}</Button>

      <LoadingState v-if="loading" />

      <div v-else-if="error" class="text-xs text-err">{{ error }}</div>

      <!-- 结果卡 -->
      <div
        v-else-if="result"
        class="bg-surface rounded-lg border border-line p-3 space-y-2 text-xs"
      >
        <div
          v-if="result.director_reasoning || result.writer_reasoning || result.editor_reasoning || result.subagents?.some(sa => sa.reasoning_content)"
          class="rounded-md border border-warn/30 bg-warn/10 px-2 py-1.5 text-warn"
        >
          Reasoning 为供应商返回的原始审计数据，可能包含系统提示、检索材料或角色私密知识；仅在此显式查看。
        </div>
        <div v-if="result.scene_brief" class="text-ink">
          <span class="text-ink-soft">场景：</span>{{ result.scene_brief }}
        </div>
        <div class="flex flex-wrap gap-1.5">
          <Badge v-if="result.profile_id" variant="neutral" size="sm">
            Profile: {{ result.profile_id }}
          </Badge>
          <Badge v-if="result.seed !== undefined && result.seed !== null" variant="neutral" size="sm">
            Seed: {{ result.seed }}
          </Badge>
        </div>
        <div v-if="result.last_hint" class="text-ink-soft">
          <span class="text-ink-soft">Hint：</span>{{ result.last_hint }}
        </div>

        <details v-if="result.director_reasoning" class="mt-2">
          <summary class="cursor-pointer text-ink-soft hover:text-ink">导演 reasoning</summary>
          <div class="mt-1.5">
            <CodeBlock :code="result.director_reasoning" language="text" wrap />
          </div>
        </details>

        <details v-if="result.editor_reasoning" class="mt-2">
          <summary class="cursor-pointer text-ink-soft hover:text-ink">编剧 reasoning</summary>
          <div class="mt-1.5">
            <CodeBlock :code="result.editor_reasoning" language="text" wrap />
          </div>
        </details>

        <details v-if="result.writer_reasoning" class="mt-2">
          <summary class="cursor-pointer text-ink-soft hover:text-ink">执笔者 reasoning</summary>
          <div class="mt-1.5">
            <CodeBlock :code="result.writer_reasoning" language="text" wrap />
          </div>
        </details>

        <!-- 子 Agent -->
        <div v-if="result.subagents && result.subagents.length > 0">
          <div class="font-medium text-ink mb-1">子 Agent</div>
          <div class="space-y-1.5">
            <div
              v-for="(sa, si) in result.subagents"
              :key="si"
              class="bg-surface-2 rounded px-2 py-1.5 border border-line"
            >
              <div class="text-ink font-medium">{{ sa.display_name || ('Agent ' + (si + 1)) }}</div>
              <div v-if="sa.task_brief" class="text-ink-soft mt-0.5">{{ sa.task_brief }}</div>
              <div v-if="sa.output_preview" class="text-ink-soft mt-1 line-clamp-3">{{ sa.output_preview }}</div>
              <details v-if="sa.reasoning_content" class="mt-1.5">
                <summary class="cursor-pointer text-ink-soft hover:text-ink">查看 reasoning</summary>
                <div class="mt-1.5">
                  <CodeBlock :code="sa.reasoning_content" language="text" wrap />
                </div>
              </details>
            </div>
          </div>
        </div>

        <!-- 完整 trace（溯源 JSON，折叠） -->
        <details class="mt-2">
          <summary class="cursor-pointer text-ink-soft text-xs hover:text-ink">查看原始 Trace</summary>
          <div class="mt-1.5">
            <CodeBlock
              :code="JSON.stringify(result, null, 2)"
              language="json"
              wrap
            />
          </div>
        </details>
      </div>
    </template>
  </div>
</template>
