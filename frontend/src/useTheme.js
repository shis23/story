import { ref, watch } from 'vue'

const STORAGE_KEY = 'storyforge-theme'
// 默认深色（现代质感基调）；用户切换后记住选择
const theme = ref(localStorage.getItem(STORAGE_KEY) || 'dark')

// 应用到 <html> + 同步手机状态栏配色
function applyTheme(t) {
  const root = document.documentElement
  if (t === 'dark') root.classList.add('dark')
  else root.classList.remove('dark')
  // 手机浏览器顶栏配色跟着主题变
  const meta = document.querySelector('meta[name="theme-color"]')
  if (meta) meta.setAttribute('content', t === 'dark' ? '#0a0a0f' : '#7c6aef')
}

// 初始化时立即应用（与 index.html 内联脚本呼应，双保险）
applyTheme(theme.value)

// 注册一次 watcher，避免每次 useTheme() 调用重复注册
watch(theme, (t) => {
  applyTheme(t)
  localStorage.setItem(STORAGE_KEY, t)
})

export function useTheme() {
  function toggle() {
    theme.value = theme.value === 'dark' ? 'light' : 'dark'
  }

  return { theme, toggle }
}
