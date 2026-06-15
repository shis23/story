<script setup>
import { ref, onMounted } from 'vue'
import { listCharacters, deleteCharacter, getCharacter } from '../tauri-api.js'

const props = defineProps({
  activeId: { type: String, default: null },
})
const emit = defineEmits(['select', 'close'])

const characters = ref([])
const loading = ref(false)

onMounted(async () => {
  await refresh()
})

async function refresh() {
  loading.value = true
  try {
    characters.value = await listCharacters()
  } catch (e) {
    console.error('加载角色列表失败:', e)
  }
  loading.value = false
}

async function handleSelect(char) {
  try {
    const detail = await getCharacter(char.id)
    emit('select', { ...char, ...detail })
  } catch (e) {
    console.error('获取角色详情失败:', e)
  }
}

async function handleDelete(char, event) {
  event.stopPropagation()
  if (!confirm(`确定删除「${char.name}」？`)) return
  try {
    await deleteCharacter(char.id)
    await refresh()
    // 如果删除的是当前选中的，清空
    if (char.id === props.activeId) {
      emit('select', null)
    }
  } catch (e) {
    console.error('删除失败:', e)
  }
}
</script>

<template>
  <div class="fixed inset-0 z-40 bg-black/40 backdrop-blur-sm flex items-center justify-center p-2" @click.self="emit('close')">
    <div class="bg-surface w-full max-w-md max-h-[80vh] rounded-2xl overflow-hidden flex flex-col">
      <!-- 顶栏 -->
      <div class="px-4 py-3 border-b border-line flex items-center gap-2 shrink-0">
        <button @click="emit('close')" class="text-ink-soft hover:text-ink text-sm">← 返回</button>
        <span class="flex-1 text-center font-medium text-ink">角色卡列表</span>
        <span class="text-xs text-ink-soft">{{ characters.length }} 张</span>
      </div>

      <!-- 列表 -->
      <div class="flex-1 overflow-y-auto">
        <div v-if="loading" class="p-8 text-center text-ink-soft text-sm">加载中...</div>

        <div v-else-if="characters.length === 0" class="p-8 text-center text-ink-soft text-sm">
          还没有导入角色卡<br>
          <span class="text-xs mt-1 block">点顶栏 📥 导入开始</span>
        </div>

        <div v-else class="divide-y divide-line">
          <div
            v-for="char in characters"
            :key="char.id"
            @click="handleSelect(char)"
            class="px-4 py-3 cursor-pointer hover:bg-accent-soft/50 transition-colors flex items-start gap-3"
            :class="char.id === activeId ? 'bg-accent-soft/50 border-l-2 border-accent' : ''"
          >
            <!-- 头像占位 -->
            <div class="w-10 h-10 rounded-full bg-bg flex items-center justify-center text-lg shrink-0">
              {{ char.name?.charAt(0) || '?' }}
            </div>

            <!-- 信息 -->
            <div class="flex-1 min-w-0">
              <div class="font-medium text-ink text-sm truncate">{{ char.name }}</div>
              <div class="text-xs text-ink-soft truncate mt-0.5">{{ char.description }}</div>
              <div class="flex items-center gap-2 mt-1">
                <span class="text-[10px] text-ink-soft/70">ST {{ char.spec_version }}</span>
                <span v-if="char.world_info_count > 0" class="text-[10px] text-ink-soft/70">📖 {{ char.world_info_count }}</span>
                <span class="text-[10px] text-ink-soft/50 ml-auto">{{ char.imported_at }}</span>
              </div>
            </div>

            <!-- 删除按钮 -->
            <button
              @click="handleDelete(char, $event)"
              class="text-ink-soft/40 hover:text-err text-xs px-1 py-0.5 rounded shrink-0"
              title="删除"
            >
              ✕
            </button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
