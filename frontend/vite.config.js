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
  },
})
