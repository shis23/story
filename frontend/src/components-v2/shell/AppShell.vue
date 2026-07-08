<script setup>
import { useUiStore } from '../../stores/index.js'
import TopBar from './TopBar.vue'
import PrimarySidebar from './PrimarySidebar.vue'
import InspectorDrawer from './InspectorDrawer.vue'
import Overlay from '../ui/Overlay.vue'

const ui = useUiStore()
</script>

<template>
  <div class="h-screen flex flex-col bg-bg overflow-hidden">
    <!-- 三栏主体 -->
    <div class="flex-1 flex min-h-0">
      <!-- 中写作区 -->
      <main class="flex-1 flex flex-col min-w-0">
        <TopBar />

        <!-- 中间内容(由父级 slot 注入:write/history/overview 视图) -->
        <div class="flex-1 overflow-y-auto">
          <slot name="content" />
        </div>

        <!-- Composer slot(底部输入栏) -->
        <slot name="composer" />
      </main>

      <!-- 右调试抽屉(桌面常驻用 v-if,移动用 overlay) -->
      <aside v-if="ui.powerMode" class="hidden lg:block">
        <InspectorDrawer />
      </aside>
    </div>

    <!-- 左导航抽屉(overlay) -->
    <Overlay :show="ui.showSidebar" side="left" :show-close="false" @update:show="ui.showSidebar = $event">
      <PrimarySidebar
        @close="ui.showSidebar = false"
        @new-campaign="$emit('new-campaign')"
        @view-history="ui.viewHistory()"
        @open-campaign="ui.showCampaignPanel = true"
        @open-char-list="ui.showCharList = true"
        @import="$emit('import')"
        @open-conn="ui.showConnConfig = true"
        @open-preset="ui.showPresetPanel = true"
        @open-plugin="ui.showPluginPanel = true"
        @open-agent-profile="ui.showAgentProfile = true"
        @open-meta="ui.showMetaPanel = true"
      />
    </Overlay>

    <!-- 右调试抽屉(overlay,移动端 + power mode) -->
    <Overlay :show="ui.showDebugDrawer" side="right" :show-close="false" @update:show="ui.showDebugDrawer = $event">
      <InspectorDrawer />
    </Overlay>

    <!-- 功能面板 slot(各管理面板,由父级注入) -->
    <slot name="panels" />
  </div>
</template>
