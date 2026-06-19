import { onMounted, onBeforeUnmount } from 'vue'

/**
 * 点击外部检测 composable。
 * @param {import('vue').Ref} targetRef  要保护的元素 ref（点击它内部不触发）
 * @param {Function} handler              点击外部时的回调
 * @param {object} [opts]
 * @param {boolean} [opts.enabled=true]   是否启用（可用于 v-model 关闭时停止监听）
 * @param {string} [opts.event='pointerdown'] 监听事件，pointerdown 比 click 更早且不受拖拽影响
 *
 * 用法：
 *   const el = ref(null)
 *   useClickOutside(el, () => { open.value = false })
 *   // <div ref="el">...</div>
 */
export function useClickOutside(targetRef, handler, opts = {}) {
  const { enabled = true, event = 'pointerdown' } = opts

  let active = enabled
  function setEnabled(v) { active = v }

  function listener(e) {
    if (!active) return
    const el = targetRef.value
    if (!el) return
    // 点击在目标元素内部 → 不处理
    if (el.contains(e.target)) return
    handler(e)
  }

  onMounted(() => {
    // capture: 先于目标内部 stopPropagation 捕获，避免内部 @click.stop 拦截掉关闭逻辑
    document.addEventListener(event, listener, true)
  })
  onBeforeUnmount(() => {
    document.removeEventListener(event, listener, true)
  })

  return { setEnabled }
}
