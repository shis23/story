import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import CardShellHost from '../../src/components/CardShellHost.vue'

/**
 * 沙箱等价性（消息壳原地渲染的前置约束）：
 * 内联 `:html` 路径必须与 `.load(:url)` 路径过同一套包装——blob 文档 +
 * sandbox 属性 + CSP 注入。两条路径在 loadShell 里共用同一个
 * prepareShellDocument/wrapRemoteHtml 调用点（结构保证）；本测试对 html
 * 路径具体断言包装产物，对两种模式断言 iframe sandbox 属性一致。
 */

const blobBodies = []
let OrigBlob

beforeEach(() => {
  OrigBlob = globalThis.Blob
  globalThis.Blob = class extends OrigBlob {
    constructor(parts, opts) {
      super(parts, opts)
      try {
        blobBodies.push(String((parts || [])[0]))
      } catch {
        /* ignore non-string parts */
      }
    }
  }
  if (typeof URL.createObjectURL !== 'function') {
    URL.createObjectURL = () => `blob:mock-${blobBodies.length}`
  }
  if (typeof URL.revokeObjectURL !== 'function') {
    URL.revokeObjectURL = () => {}
  }
})

afterEach(() => {
  globalThis.Blob = OrigBlob
  blobBodies.length = 0
})

describe('CardShellHost sandbox equivalence (inline html vs remote url)', () => {
  it('wraps inline html into a CSP-pinned bridge document inside a sandboxed iframe', async () => {
    const wrapper = mount(CardShellHost, {
      props: { html: '<body><div id="app"></div><script>boot()<' + '/script></body>' },
    })
    await flushPromises()

    const iframe = wrapper.find('iframe')
    expect(iframe.exists()).toBe(true)
    expect(iframe.attributes('sandbox')).toBe('allow-scripts')

    const doc = blobBodies.find((b) => b.includes('boot()'))
    expect(doc).toBeTruthy()
    // M3 CSP 注入：与 .load 壳同一 buildShellCspMetaTag
    expect(doc).toContain('Content-Security-Policy')
    expect(doc).toContain("default-src 'none'")
    // bridge 注入：与 .load 壳同一 wrapRemoteHtml
    expect(doc).toContain('__sfShellBridgeSession')

    wrapper.unmount()
  })

  it('remote url mode uses the identical iframe sandbox attribute', async () => {
    const wrapper = mount(CardShellHost, {
      props: { url: 'https://example.com/shell/index.html' },
    })
    await flushPromises()

    // 非 Tauri 环境 fetch 会失败进错误分支，但 iframe 元素与 sandbox
    // 属性是模板静态的，两种模式共用同一个元素定义。
    expect(wrapper.find('iframe').attributes('sandbox')).toBe('allow-scripts')

    wrapper.unmount()
  })
})
