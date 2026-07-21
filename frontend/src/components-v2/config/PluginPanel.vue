<script setup>
import { ref, onMounted } from 'vue'
import { confirmDialog, alertDialog } from '../../components/base/BaseDialog.js'
import { listPlugins, installPlugin, uninstallPlugin, setPluginEnabled } from '../../tauri-api.js'
import PanelHost from '../shell/PanelHost.vue'
import Button from '../ui/Button.vue'
import Badge from '../ui/Badge.vue'
import Toggle from '../ui/Toggle.vue'
import DataList from '../ui/DataList.vue'
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
  <PanelHost :show="true" title="插件管理" side="left" @close="emit('close')">
    <template #header>
      <div class="flex items-center justify-between min-w-0 gap-2">
        <h2 class="text-sm font-semibold text-ink truncate">插件管理</h2>
        <Button variant="default" size="sm" @click="showInstall = !showInstall">
          {{ showInstall ? '取消' : '安装插件' }}
        </Button>
      </div>
    </template>

    <!-- 安装区域 -->
    <div v-if="showInstall" class="px-4 py-3 border-b border-line bg-surface/50 space-y-2">
      <div class="text-sm text-ink-soft">粘贴插件 manifest JSON：</div>
      <textarea
        v-model="installJson"
        class="w-full h-32 text-xs font-mono p-2 rounded border border-line bg-bg resize-none focus:outline-none focus:border-accent"
        placeholder='{ "id": "my-plugin", "name": "My Plugin", "version": "1.0.0", "entry_html": "<h1>Hello</h1>", ... }'
      />
      <div class="flex justify-between items-center">
        <div v-if="installError" class="text-xs text-err">{{ installError }}</div>
        <div v-else-if="installSuccess" class="text-xs text-ok">✓ 安装成功</div>
        <Button
          variant="primary"
          size="md"
          class="ml-auto"
          :loading="installing"
          :disabled="!installJson.trim()"
          @click="doInstall"
        >{{ installing ? '安装中…' : '安装' }}</Button>
      </div>
    </div>

    <!-- 插件列表 -->
    <div class="p-4">
      <LoadingState v-if="loading" />

      <EmptyState
        v-else-if="plugins.length === 0"
        title="暂无插件"
        description="点击上方「安装插件」添加"
      >
        <template #icon><span class="text-2xl">🧩</span></template>
      </EmptyState>

      <!-- 用 DataList 统一渲染插件项（插槽自定义） -->
      <DataList
        v-else
        :items="plugins"
        active-key="id"
      >
        <template #item="{ item }">
          <div class="border border-line rounded-lg p-3 bg-surface">
            <div class="flex items-start justify-between gap-2">
              <div class="flex-1 min-w-0">
                <div class="flex items-center gap-2 flex-wrap">
                  <span class="font-medium text-ink">{{ item.name }}</span>
                  <Badge variant="neutral" size="sm">v{{ item.version }}</Badge>
                  <Badge
                    :variant="item.enabled ? 'ok' : 'neutral'"
                    size="sm"
                  >{{ item.enabled ? '已启用' : '已禁用' }}</Badge>
                </div>
                <div v-if="item.description" class="text-xs text-ink-soft mt-1 line-clamp-2">{{ item.description }}</div>
                <div v-if="item.author" class="text-xs text-ink-faint mt-0.5">作者: {{ item.author }}</div>
              </div>
              <div class="flex items-center gap-2 shrink-0">
                <Toggle
                  :model-value="item.enabled"
                  @update:model-value="toggleEnabled(item)"
                />
                <Button
                  variant="danger"
                  size="sm"
                  @click="doUninstall(item)"
                >卸载</Button>
              </div>
            </div>

            <!-- 权限标签 -->
            <div v-if="item.permissions?.length" class="flex flex-wrap gap-1 mt-2">
              <Badge
                v-for="perm in item.permissions"
                :key="perm"
                variant="accent"
                size="sm"
              >{{ perm }}</Badge>
            </div>

            <!-- UI 挂载点标签 -->
            <div v-if="item.ui_slots?.length" class="flex flex-wrap gap-1 mt-1">
              <Badge
                v-for="slot in item.ui_slots"
                :key="slot"
                variant="neutral"
                size="sm"
              >📐 {{ slot }}</Badge>
            </div>
          </div>
        </template>
      </DataList>
    </div>
  </PanelHost>
</template>
