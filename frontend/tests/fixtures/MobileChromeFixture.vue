<script setup>
import { ref } from 'vue'
import AppFrame from '../../src/design/shell/AppFrame.vue'
import TopBar from '../../src/components-v2/shell/TopBar.vue'
import PrimarySidebar from '../../src/components-v2/shell/PrimarySidebar.vue'
import PanelHost from '../../src/components-v2/shell/PanelHost.vue'
import BaseOverlay from '../../src/components/base/BaseOverlay.vue'
import CampaignScreen from '../../src/design/campaign/CampaignScreen.vue'
import WritingScreen from '../../src/design/writing/WritingScreen.vue'
import { fxMessagesWritten, fxPipelineDone, fxPipelineStreaming } from '../../src/design/writing/fixtures.js'
import { useCampaignStore, useUiStore, useWritingStore } from '../../src/stores/index.js'

const ui = useUiStore()
const campaign = useCampaignStore()
const writing = useWritingStore()
const params = new URLSearchParams(location.search)
const writingScene = ref(params.get('scene') === 'writing')
campaign.activeCampaign = {
  id: 'mobile-chrome-campaign',
  name: params.get('title') || (writingScene.value ? '雨巷 · 第一章' : 'USB 真机验收 3b114fe 长标题布局回归'),
  story_clock: '第1天',
}
const selected = campaign.activeCampaign
const panelOpen = ref(false)
const legacyOpen = ref(false)
const action = ref('')
const generationMode = ref('continuation')
</script>

<template>
  <AppFrame
    :sidebar-open="ui.showSidebar"
    :sidebar-collapsed="ui.sidebarCollapsed"
    :inspector-open="ui.showDebugDrawer"
    @update:sidebar-open="ui.showSidebar = $event"
    @update:sidebar-collapsed="ui.sidebarCollapsed = $event"
    @update:inspector-open="ui.showDebugDrawer = $event"
  >
    <template #topbar="{ docked, sidebarVisible, toggleSidebar }">
      <TopBar :sidebar-docked="docked" :sidebar-visible="sidebarVisible" @toggle-sidebar="toggleSidebar" />
    </template>
    <template #sidebar="{ docked, collapseSidebar }">
      <PrimarySidebar :docked="docked" @collapse="collapseSidebar" @close="ui.showSidebar = false" @open-campaign="ui.showCampaignPanel = true" />
    </template>
    <template #content>
      <div v-if="!writingScene" class="flex flex-col gap-3 p-4">
        <button @click="panelOpen = true">测试功能面板</button>
        <button @click="legacyOpen = true">测试旧版面板</button>
        <button @click="writing.isWriting = !writing.isWriting">切换生成状态</button>
        <button @click="writingScene = true">查看写作界面</button>
        <output>{{ action }}</output>
      </div>
      <WritingScreen
        v-else
        class="sf-view-enter"
        :title="selected.name"
        :messages="fxMessagesWritten"
        :pipeline="writing.isWriting ? fxPipelineStreaming : fxPipelineDone"
        :is-writing="writing.isWriting"
        :show-pipeline="writing.isWriting"
        :show-generation-modes="true"
        :generation-mode="generationMode"
        @update-generation-mode="generationMode = $event"
        @start-writing="writing.isWriting = true"
        @cancel="writing.isWriting = false"
      />
    </template>
    <template #inspector>
      <header class="sf-toolbar flex items-center justify-between px-4">
        <span>调试</span>
        <button aria-label="关闭调试" @click="ui.showDebugDrawer = false">关闭</button>
      </header>
    </template>
    <template #panels>
      <PanelHost :show="ui.showCampaignPanel" side="full" :show-chrome="false" :body-scroll="false" @close="ui.showCampaignPanel = false">
        <CampaignScreen
          :selected-campaign="selected"
          :selected-campaign-id="selected.id"
          :campaigns="[selected]"
          @close="ui.showCampaignPanel = false"
          @export-st="action = 'export-st'"
          @export-bundle="action = 'export-bundle'"
          @import-bundle="action = 'import-bundle'"
        >
          <template #detail><p>验收内容</p></template>
        </CampaignScreen>
      </PanelHost>
      <PanelHost :show="panelOpen" title="功能面板" @close="panelOpen = false">
        <label class="block p-4">名称<input class="block border" aria-label="面板输入" /></label>
      </PanelHost>
      <BaseOverlay v-model="legacyOpen" title="旧版面板" position="right" size="drawer">
        <p class="p-4">验收内容</p>
      </BaseOverlay>
    </template>
  </AppFrame>
</template>
