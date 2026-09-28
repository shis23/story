import { createApp } from 'vue'
import { createPinia } from 'pinia'
import './style.css'
import AppV2 from './AppV2.vue'

// 设计预览通道（fixture 驱动，无 store 依赖）。
// F-31：只在开发态注册 —— 生产构建里这些分支会把三个 Demo 组件
// （+ fixture）打进产物，任何用户改成 `#design-*` 哈希都能绕过 AppV2 拿到
// 一个没有 store/权限门的空壳页面。`import.meta.env.DEV` 由 Vite 静态替换，
// 生产构建下整段分支连同动态 import 一起被 tree-shake 掉。
const DESIGN_PREVIEW_ROUTES = {
  '#design-writing': () => import('./design/writing/WritingScreenDemo.vue'),
  '#design-campaign': () => import('./design/campaign/CampaignScreenDemo.vue'),
  '#design-meta': () => import('./design/meta/MetaScreenDemo.vue'),
}

const previewLoader = import.meta.env.DEV ? DESIGN_PREVIEW_ROUTES[location.hash] : null

if (previewLoader) {
  previewLoader().then(({ default: Demo }) => {
    createApp(Demo).mount('#app')
  })
} else {
  createApp(AppV2).use(createPinia()).mount('#app')
}
