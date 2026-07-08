import { createApp } from 'vue'
import { createPinia } from 'pinia'
import './style.css'
import AppV2 from './AppV2.vue'

// Phase 8:挂载 AppV2 + Pinia。
// 旧 App.vue 保留(阶段 9 才删);回退只需把下一行 import 改为 `import App from './App.vue'`
// 并将 createApp(AppV2) 改为 createApp(App) 即可。
createApp(AppV2).use(createPinia()).mount('#app')
