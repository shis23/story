<script setup>
import { ref, onMounted } from 'vue'
import { confirmDialog, alertDialog } from './base/BaseDialog.js'
import BaseOverlay from './base/BaseOverlay.vue'
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
  const ok = await confirmDialog(`确定卸载插件「${plugin.name}」？`, { title: '卸载确认' })
  if (!ok) return
  try {
    await uninstallPlugin(plugin.id)
    await loadPlugins()
  } catch (e) {
    await alertDialog('卸载失败: ' + e)
  }
}

async function toggleEnabled(plugin) {
  try {
    await setPluginEnabled(plugin.id, !plugin.enabled)
    await loadPlugins()
  } catch (e) {
    await alertDialog('操作失败: ' + e)
  }
}

onMounted(loadPlugins)
</script>

<template>
  <BaseOverlay :model-value="true" title="🔌 插件管理" size="lg" position="left" @close="$emit('close')">
    <template #header-extra>
      <button class="min-h-[44px] px-3 text-sm rounded-lg bg-bg text-ink-soft hover:bg-line transition-colors" @click="showInstall = !showInstall">
        {{ showInstall ? '取消' : '📦 安装插件' }}
      </button>
    </template>

    <!-- 安装区域 -->
    <div v-if="showInstall" class="px-4 py-3 border-b border-line bg-surface/50">
      <div class="text-sm text-ink-soft mb-2">粘贴插件 manifest JSON：</div>
      <textarea
        v-model="installJson"
        class="w-full h-32 text-xs font-mono p-2 rounded border border-line bg-bg resize-none focus:outline-none focus:border-accent"
        placeholder='{ "id": "my-plugin", "name": "My Plugin", "version": "1.0.0", "entry_html": "<h1>Hello</h1>", ... }'
      />
      <div class="flex justify-between items-center mt-2">
        <div v-if="installError" class="text-xs text-err">{{ installError }}</div>
        <div v-if="installSuccess" class="text-xs text-ok">✓ 安装成功</div>
        <button
          class="min-h-[44px] px-4 text-sm rounded-lg bg-accent text-white hover:opacity-90 disabled:opacity-50 transition-colors ml-auto"
          :disabled="!installJson.trim() || installing"
          @click="doInstall"
        >
          {{ installing ? '安装中…' : '安装' }}
        </button>
      </div>
    </div>

    <!-- 插件列表 -->
    <div class="p-4">
      <div v-if="loading" class="text-center text-ink-soft py-8">加载中…</div>
      <div v-else-if="plugins.length === 0" class="text-center text-ink-soft py-8">
        <div class="text-3xl mb-2">🧩</div>
        <div>暂无插件</div>
        <div class="text-xs mt-1">点击上方「安装插件」添加</div>
      </div>
      <div v-else class="space-y-3">
        <div
          v-for="plugin in plugins"
          :key="plugin.id"
          class="border border-line rounded-lg p-3 bg-surface"
        >
          <div class="flex items-start justify-between gap-2">
            <div class="flex-1 min-w-0">
              <div class="flex items-center gap-2">
                <span class="font-medium text-ink">{{ plugin.name }}</span>
                <span class="text-xs text-ink-soft">v{{ plugin.version }}</span>
                <span
                  class="text-xs px-1.5 py-0.5 rounded"
                  :class="plugin.enabled ? 'bg-ok/10 text-ok' : 'bg-ink-soft/10 text-ink-soft'"
                >
                  {{ plugin.enabled ? '已启用' : '已禁用' }}
                </span>
              </div>
              <div v-if="plugin.description" class="text-xs text-ink-soft mt-1 line-clamp-2">{{ plugin.description }}</div>
              <div v-if="plugin.author" class="text-xs text-ink-soft/70 mt-0.5">作者: {{ plugin.author }}</div>
            </div>
            <div class="flex flex-col gap-1.5 shrink-0">
              <button
                class="min-h-[36px] px-3 text-xs rounded-lg transition-colors"
                :class="plugin.enabled ? 'bg-warn/10 text-warn hover:bg-warn/20' : 'bg-ok/10 text-ok hover:bg-ok/20'"
                @click="toggleEnabled(plugin)"
              >
                {{ plugin.enabled ? '禁用' : '启用' }}
              </button>
              <button
                class="min-h-[36px] px-3 text-xs rounded-lg bg-err/10 text-err hover:bg-err/20 transition-colors"
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
              class="text-xs px-1.5 py-0.5 rounded bg-accent-soft text-accent"
            >
              {{ perm }}
            </span>
          </div>

          <!-- UI 挂载点标签 -->
          <div v-if="plugin.ui_slots?.length" class="flex flex-wrap gap-1 mt-1">
            <span
              v-for="slot in plugin.ui_slots"
              :key="slot"
              class="text-xs px-1.5 py-0.5 rounded bg-running/10 text-running"
            >
              📐 {{ slot }}
            </span>
          </div>
        </div>
      </div>
    </div>
  </BaseOverlay>
</template>
