import { createApp, h, ref } from 'vue'
import BaseOverlay from './BaseOverlay.vue'

/**
 * 统一对话框入口，替换散落的 alert() / window.confirm / window.prompt。
 *
 * - confirm / alert：薄封装 @tauri-apps/plugin-dialog 的 ask / message（原生体验）。
 * - prompt：Tauri 无原生 prompt，用 BaseOverlay 渲染输入弹层（命令式调用，返回 Promise<string|null>）。
 *
 * 用法：
 *   import { confirmDialog, alertDialog, promptDialog } from './base/BaseDialog.js'
 *   if (await confirmDialog('确定删除？')) { ... }
 *   const name = await promptDialog('输入预设名', '默认值')
 *   if (name === null) return  // 用户取消
 *   await alertDialog('保存失败: ' + e)
 */

// ─── confirm / alert：原生 Tauri 对话框 ────────────────────────────────────

export async function confirmDialog(message, opts = {}) {
  const { ask } = await import('@tauri-apps/plugin-dialog')
  return await ask(message, {
    title: opts.title || '确认',
    kind: opts.kind || 'warning',
    ...opts,
  })
}

export async function alertDialog(message, opts = {}) {
  const { message: msg } = await import('@tauri-apps/plugin-dialog')
  return await msg(message, {
    title: opts.title || '提示',
    ...opts,
  })
}

// ─── prompt：BaseOverlay + 输入框（命令式） ─────────────────────────────────
//
// Tauri 无原生 prompt；用临时 Vue 应用挂一个 BaseOverlay 输入弹层，
// 返回 Promise<string|null>：null=取消，字符串=用户输入（已 trim）。

export function promptDialog(message, defaultValue = '', opts = {}) {
  return new Promise((resolve) => {
    const host = document.createElement('div')
    document.body.appendChild(host)

    const app = createApp({
      setup() {
        const visible = ref(true)
        const value = ref(defaultValue)

        function done(result) {
          visible.value = false
          // 等退场动画结束再卸载
          setTimeout(() => {
            app.unmount()
            host.remove()
            resolve(result)
          }, 220)
        }

        function confirm() {
          const v = value.value.trim()
          done(v === '' ? null : v)
        }

        return () => h(BaseOverlay, {
          modelValue: visible.value,
          'onUpdate:modelValue': (v) => { if (!v) done(null) },
          title: opts.title || '输入',
          size: 'sm',
          position: 'center',
          showHeader: true,
        }, {
          default: () => h('div', { class: 'p-4 space-y-3' }, [
            h('p', { class: 'text-sm text-ink-soft' }, message),
            h('input', {
              ref: (el) => {
                if (el) {
                  // 自动聚焦 + 选中默认值
                  requestAnimationFrame(() => { el.focus(); el.select() })
                }
              },
              value: value.value,
              onInput: (e) => { value.value = e.target.value },
              onKeydown: (e) => {
                if (e.key === 'Enter') { e.preventDefault(); confirm() }
                if (e.key === 'Escape') { done(null) }
              },
              class: 'w-full min-h-[44px] px-3 rounded-lg border border-line bg-bg text-ink focus:outline-none focus:border-accent',
              placeholder: opts.placeholder || '',
            }),
          ]),
          footer: () => h('div', { class: 'flex justify-end gap-2' }, [
            h('button', {
              class: 'min-h-[44px] px-4 rounded-lg text-sm text-ink-soft hover:bg-accent-soft transition-colors',
              onClick: () => done(null),
            }, '取消'),
            h('button', {
              class: 'min-h-[44px] px-4 rounded-lg text-sm bg-accent text-white hover:opacity-90 transition-colors',
              onClick: confirm,
            }, opts.okLabel || '确定'),
          ]),
        })
      },
    })
    app.mount(host)
  })
}
