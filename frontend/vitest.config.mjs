import { defineConfig } from 'vitest/config'
import vue from '@vitejs/plugin-vue'

// Vitest 配置——仅用于组件挂载测试(tests/components-v2/**)。
// 纯 JS 工具测试继续用 node --test(npm test),互不干扰。
// vitest 自动用 vite vue 插件编译 SFC + happy-dom 提供 DOM。
export default defineConfig({
  plugins: [vue()],
  test: {
    environment: 'happy-dom',
    include: ['tests/components-v2/**/*.test.mjs'],
  },
})
