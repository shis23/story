import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'

// Tauri v2 前端配置
// host: true 允许手机/模拟器访问开发服务器
// 端口固定 1420（Tauri 默认）

export default defineConfig({
  plugins: [vue(), tailwindcss()],
  server: {
    host: true,
    port: 1420,
    strictPort: true,
  },
  // Tauri 需要相对路径（打包后用 file:// 协议）
  base: './',
  build: {
    // Tauri 在 Windows 上用 Chromium，目标 Chrome 105+
    target: ['es2021', 'chrome105', 'safari15'],
    // 不压缩，方便调试
    minify: !process.env.TAURI_DEBUG ? 'esbuild' : false,
    // 开启 sourcemap（调试用）
    sourcemap: !!process.env.TAURI_DEBUG,
    rollupOptions: {
      output: {
        // 框架依赖拆 vendor：应用代码迭代不再重刷框架 chunk 缓存。
        // 主 chunk 674K → 529K（vendor-vue 133K + vendor-sanitize 29K 拆出）。
        manualChunks: {
          'vendor-vue': ['vue', 'pinia', '@headlessui/vue'],
          'vendor-sanitize': ['dompurify'],
        },
      },
    },
    // 529K 为应用代码本体（Tauri 本地加载，无网络传输代价）；
    // 继续压需异步组件化，收益不抵回归风险——有意接受。
    chunkSizeWarningLimit: 560,
  },
})
