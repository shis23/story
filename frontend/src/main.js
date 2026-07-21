import { createApp } from 'vue'
import { createPinia } from 'pinia'
import './style.css'
import AppV2 from './AppV2.vue'

// 设计预览通道：#design-writing 直达重设计写作屏（fixture 驱动，无 store 依赖）。
// 仅开发预览用；正式接线完成后由接线 agent 移除本分支。
if (location.hash === '#design-writing') {
  import('./design/writing/WritingScreenDemo.vue').then(({ default: WritingScreenDemo }) => {
    createApp(WritingScreenDemo).mount('#app')
  })
} else {
  createApp(AppV2).use(createPinia()).mount('#app')
}
