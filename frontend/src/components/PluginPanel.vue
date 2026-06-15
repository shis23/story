<template>
  <div class="fixed inset-0 z-50 bg-black/40 backdrop-blur-sm flex items-end sm:items-center justify-center" @click.self="$emit('close')">
    <div class="bg-white dark:bg-zinc-900 rounded-t-2xl sm:rounded-2xl w-full max-w-2xl max-h-[80vh] flex flex-col shadow-2xl">
      <!-- 头部 -->
      <div class="flex items-center justify-between px-4 py-3 border-b border-zinc-200 dark:border-zinc-700">
        <div class="font-semibold text-lg">🔌 插件管理</div>
        <div class="flex gap-2">
          <button class="px-3 py-1 text-sm rounded bg-zinc-100 dark:bg-zinc-700 hover:bg-zinc-200 dark:hover:bg-zinc-600" @click="showInstall = !showInstall">
            {{ showInstall ? '取消' : '📦 安装插件' }}
          </button>
          <button class="px-3 py-1 text-sm rounded bg-zinc-100 dark:bg-zinc-700 hover:bg-zinc-200 dark:hover:bg-zinc-600" @click="$emit('close')">✕</button>
        </div>
      </div>

      <!-- 安装区域 -->
      <div v-if="showInstall" class="px-4 py-3 border-b border-zinc-200 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-800/50">
        <div class="text-sm text-zinc-500 mb-2">粘贴插件 manifest JSON：</div>
        <textarea
          v-model="installJson"
          class="w-full h-32 text-xs font-mono p-2 rounded border border-zinc-300 dark:border-zinc-600 bg-white dark:bg-zinc-800 resize-none"
          placeholder='{ "id": "my-plugin", "name": "My Plugin", "version": "1.0.0", "entry_html": "<h1>Hello</h1>", ... }'
        />
        <div class="flex justify-between items-center mt-2">
          <div v-if="installError" class="text-xs text-red-500">{{ installError }}</div>
          <div v-if="installSuccess" class="text-xs text-green-500">✓ 安装成功</div>
          <button
            class="px-3 py-1 text-sm rounded bg-emerald-500 text-white hover:bg-emerald-600 disabled:opacity-50"
            :disabled="!installJson.trim() || installing"
            @click="doInstall"
          >
            {{ installing ? '安装中...' : '安装' }}
          </button>
        </div>
      </div>

      <!-- 插件列表 -->
      <div class="flex-1 overflow-y-auto p-4">
        <div v-if="loading" class="text-center text-zinc-400 py-8">加载中...</div>
        <div v-else-if="plugins.length === 0" class="text-center text-zinc-400 py-8">
          <div class="text-3xl mb-2">🧩</div>
          <div>暂无插件</div>
          <div class="text-xs mt-1">点击上方「安装插件」添加</div>
        </div>
        <div v-else class="space-y-3">
          <div
            v-for="plugin in plugins"
            :key="plugin.id"
            class="border border-zinc-200 dark:border-zinc-700 rounded-lg p-3"
          >
            <div class="flex items-start justify-between">
              <div class="flex-1 min-w-0">
                <div class="flex items-center gap-2">
                  <span class="font-medium">{{ plugin.name }}</span>
                  <span class="text-xs text-zinc-400">v{{ plugin.version }}</span>
                  <span
                    class="text-xs px-1.5 py-0.5 rounded"
                    :class="plugin.enabled ? 'bg-emerald-100 text-emerald-700 dark:bg-emerald-900/30 dark:text-emerald-400' : 'bg-zinc-100 text-zinc-500 dark:bg-zinc-700 dark:text-zinc-400'"
                  >
                    {{ plugin.enabled ? '已启用' : '已禁用' }}
                  </span>
                </div>
                <div v-if="plugin.description" class="text-xs text-zinc-500 mt-1 line-clamp-2">{{ plugin.description }}</div>
                <div v-if="plugin.author" class="text-xs text-zinc-400 mt-0.5">作者: {{ plugin.author }}</div>
              </div>
              <div class="flex gap-1.5 ml-2 shrink-0">
                <button
                  class="px-2 py-1 text-xs rounded"
                  :class="plugin.enabled ? 'bg-amber-100 text-amber-700 hover:bg-amber-200 dark:bg-amber-900/30 dark:text-amber-400' : 'bg-emerald-100 text-emerald-700 hover:bg-emerald-200 dark:bg-emerald-900/30 dark:text-emerald-400'"
                  @click="toggleEnabled(plugin)"
                >
                  {{ plugin.enabled ? '禁用' : '启用' }}
                </button>
                <button
                  class="px-2 py-1 text-xs rounded bg-red-100 text-red-600 hover:bg-red-200 dark:bg-red-900/30 dark:text-red-400"
                  @click="doUninstall(plugin)"
                >
                  卸载
                </button>
              </div>
            </div>

            <!-- 权限标签 -->
            <div v-if="plugin.permissions?.length" class="flex flex-wrap gap-1 mt-2">
              <span
                v-for="perm in plugin.permissions"
                :key="perm"
                class="text-xs px-1.5 py-0.5 rounded bg-blue-50 text-blue-600 dark:bg-blue-900/30 dark:text-blue-400"
              >
                {{ perm }}
              </span>
            </div>

            <!-- UI 挂载点标签 -->
            <div v-if="plugin.ui_slots?.length" class="flex flex-wrap gap-1 mt-1">
              <span
                v-for="slot in plugin.ui_slots"
                :key="slot"
                class="text-xs px-1.5 py-0.5 rounded bg-purple-50 text-purple-600 dark:bg-purple-900/30 dark:text-purple-400"
              >
                📐 {{ slot }}
              </span>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { listPlugins, installPlugin, uninstallPlugin, setPluginEnabled } from '../tauri-api.js'

defineEmits(['close'])

const plugins = ref([])
const loading = ref(true)
const showInstall = ref(false)
const installJson = ref('')
const installing = ref(false)
const installError = ref('')
const installSuccess = ref(false)

async function loadPlugins() {
  loading.value = true
  try {
    plugins.value = await listPlugins()
  } catch (e) {
    console.error('加载插件列表失败:', e)
  } finally {
    loading.value = false
  }
}

async function doInstall() {
  installError.value = ''
  installSuccess.value = false
  installing.value = true
  try {
    await installPlugin(installJson.value)
    installSuccess.value = true
    installJson.value = ''
    await loadPlugins()
  } catch (e) {
    installError.value = String(e)
  } finally {
    installing.value = false
  }
}

async function doUninstall(plugin) {
  if (!confirm(`确定卸载插件「${plugin.name}」？`)) return
  try {
    await uninstallPlugin(plugin.id)
    await loadPlugins()
  } catch (e) {
    alert('卸载失败: ' + e)
  }
}

async function toggleEnabled(plugin) {
  try {
    await setPluginEnabled(plugin.id, !plugin.enabled)
    await loadPlugins()
  } catch (e) {
    alert('操作失败: ' + e)
  }
}

onMounted(loadPlugins)
</script>
