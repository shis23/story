import { createApp } from 'vue'
import { createPinia } from 'pinia'
import './style.css'
import AppV2 from './AppV2.vue'

createApp(AppV2).use(createPinia()).mount('#app')
