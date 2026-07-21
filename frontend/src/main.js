import { createApp } from 'vue'
import { createPinia } from 'pinia'
import './style.css'
import AppV2 from './AppV2.vue'

// 设计预览通道（fixture 驱动，无 store 依赖）。
// 正式接线并验收后删除对应分支。
const hash = location.hash
if (hash === '#design-writing') {
  import('./design/writing/WritingScreenDemo.vue').then(({ default: Demo }) => {
    createApp(Demo).mount('#app')
  })
} else if (hash === '#design-campaign') {
  import('./design/campaign/CampaignScreenDemo.vue').then(({ default: Demo }) => {
    createApp(Demo).mount('#app')
  })
} else {
  createApp(AppV2).use(createPinia()).mount('#app')
}
