<script setup>
import { ref, onMounted } from 'vue'
import { confirmDialog, alertDialog } from '../../components/base/BaseDialog.js'
import { listPlugins, installPlugin, uninstallPlugin, setPluginEnabled } from '../../tauri-api.js'
import PanelHost from '../shell/PanelHost.vue'
import Button from '../ui/Button.vue'
import Badge from '../ui/Badge.vue'
import Toggle from '../ui/Toggle.vue'
import EmptyState from '../ui/EmptyState.vue'
import LoadingState from '../ui/LoadingState.vue'

const emit = defineEmits(['close'])

const plugins = ref([])
const loading = ref(true)
const showInstall = ref(false)
const installJson = ref('')
const installing = ref(false)
const installError = ref('')
const installSuccess = ref(false)
/** 防止 Toggle 连点造成重入 */
const togglingId = ref(null)

async function loadPlugins() {
  loading.value = true
  try {
    const list = await listPlugins()
    plugins.value = Array.isArray(list) ? list : []
  } catch (e) {
    console.error('加载插件列表失败:', e)
    plugins.value = []
    // 浏览器无 Tauri 时不弹窗卡死，仅控制台
  } finally {
    loading.value = false
  }
}

async function doInstall() {
  if (installing.value) return
  installError.value = ''
  installSuccess.value = false
  installing.value = true
  try {
    await installPlugin(installJson.value)
    installSuccess.value = true
    installJson.value = ''
    showInstall.value = false
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

async function toggleEnabled(plugin, next) {
  if (togglingId.value === plugin.id) return
  togglingId.value = plugin.id
  const target = typeof next === 'boolean' ? next : !plugin.enabled
  try {
    await setPluginEnabled(plugin.id, target)
    // 乐观更新，避免整表 loading 造成「卡死」感
    const row = plugins.value.find((p) => p.id === plugin.id)
    if (row) row.enabled = target
  } catch (e) {
    await alertDialog('操作失败: ' + e)
    await loadPlugins()
  } finally {
    togglingId.value = null
  }
}

onMounted(loadPlugins)
</script>

<template>
  <PanelHost :show="true" title="插件管理" side="left" @close="emit('close')">
    <template #header>
      <div class="flex items-center justify-between min-w-0 gap-2 w-full pr-1">
        <h2 class="text-sm font-semibold text-ink truncate">插件管理</h2>
        <Button variant="default" size="sm" @click="showInstall = !showInstall">
          {{ showInstall ? '取消' : '安装插件' }}
        </Button>
      </div>
    </template>

    <!-- 安装区域 -->
    <div v-if="showInstall" class="px-4 py-3 border-b border-line bg-surface-2/40 space-y-2">
      <div class="text-sm text-ink-soft">粘贴插件 manifest JSON：</div>
      <textarea
        v-model="installJson"
        class="w-full h-32 text-xs font-mono p-2 rounded border border-line bg-bg resize-none focus:outline-none focus:border-accent"
        placeholder='{ "id": "my-plugin", "name": "My Plugin", "version": "1.0.0", "entry_html": "<h1>Hello</h1>", ... }'
      />
      <div class="flex justify-between items-center gap-2">
        <div v-if="installError" class="text-xs text-err break-words min-w-0">{{ installError }}</div>
        <div v-else-if="installSuccess" class="text-xs text-ok">安装成功</div>
        <Button
          variant="primary"
          size="md"
          class="ml-auto shrink-0"
          :loading="installing"
          :disabled="!installJson.trim() || installing"
          @click="doInstall"
        >{{ installing ? '安装中…' : '安装' }}</Button>
      </div>
    </div>

    <!-- 插件列表：不用 DataList 套卡片（避免双层边框 + 点击层干扰 Toggle） -->
    <div class="p-4 space-y-3">
      <LoadingState v-if="loading" label="加载插件…" />

      <EmptyState
        v-else-if="plugins.length === 0"
        title="暂无插件"
        description="点击上方「安装插件」添加"
      />

      <ul v-else class="space-y-2">
        <li
          v-for="item in plugins"
          :key="item.id"
          class="rounded-lg border border-line bg-surface p-3"
        >
          <div class="flex items-start justify-between gap-2">
            <div class="flex-1 min-w-0">
              <div class="flex items-center gap-2 flex-wrap">
                <span class="font-medium text-ink text-sm">{{ item.name }}</span>
                <Badge variant="neutral" size="sm">v{{ item.version }}</Badge>
                <Badge :variant="item.enabled ? 'ok' : 'neutral'" size="sm">
                  {{ item.enabled ? '已启用' : '已禁用' }}
                </Badge>
              </div>
              <div v-if="item.description" class="text-xs text-ink-soft mt-1 line-clamp-2">{{ item.description }}</div>
              <div v-if="item.author" class="text-xs text-ink-faint mt-0.5">作者: {{ item.author }}</div>
            </div>
            <div class="flex items-center gap-2 shrink-0">
              <Toggle
                :model-value="!!item.enabled"
                :disabled="togglingId === item.id"
                @update:model-value="(v) => toggleEnabled(item, v)"
              />
              <Button variant="danger" size="sm" @click="doUninstall(item)">卸载</Button>
            </div>
          </div>

          <div v-if="item.permissions?.length" class="flex flex-wrap gap-1 mt-2">
            <Badge v-for="perm in item.permissions" :key="perm" variant="accent" size="sm">{{ perm }}</Badge>
          </div>
          <div v-if="item.ui_slots?.length" class="flex flex-wrap gap-1 mt-1">
            <Badge v-for="slot in item.ui_slots" :key="slot" variant="neutral" size="sm">{{ slot }}</Badge>
          </div>
        </li>
      </ul>
    </div>
  </PanelHost>
</template>
