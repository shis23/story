import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import vm from 'node:vm'

/**
 * M-24：插件帧自我导航后的信任锚。
 *
 * `sandbox` 不阻止帧自导航（location.href=…），导航后 WindowProxy 身份不变，
 * 因此 `event.source === iframe.contentWindow` 挡不住「换了一份文档的同一帧」。
 * 现在宿主为每份文档生成一次性握手令牌并嵌入该文档的桥脚本：只有当前文档能
 * 回传令牌，`@load` 之后必须重新握手，未握手帧一律不受信。
 *
 * `@tauri-apps/api/core` 被 mock 成确定性 invoke：`card_shell_register_doc`
 * 返回合法 token ⇒ 组件不发 blob 兜底，`iframeSrc` 落在壳源上。
 */

/**
 * 定位 `PluginHost.vue` 源码，兼容真实 vitest 与单进程直跑。
 *
 * 真实 `npm run test:ui`（Vite 转换）下 `import.meta.url` **不是 `file:` scheme**，
 * 因此 `fs.readFileSync(new URL(..., import.meta.url))` 会抛
 * `TypeError: The URL must be of scheme file`（本轮 task-27 的失败根因，与
 * 被测代码无关）。这里改为：file: scheme 时用 `fileURLToPath`；否则按 cwd 解析
 * （`npm run test:ui` 的 cwd 为 `frontend/`），并保留仓库根候选兜底；全都找不到
 * 就显式报出候选路径，绝不静默跳过。
 */
function resolvePluginHostSource() {
  const relative = 'src/components/PluginHost.vue'
  const candidates = []
  if (typeof import.meta.url === 'string' && import.meta.url.startsWith('file:')) {
    candidates.push(fileURLToPath(new URL(`../../${relative}`, import.meta.url)))
  }
  candidates.push(path.resolve(process.cwd(), relative))
  candidates.push(path.resolve(process.cwd(), 'frontend', relative))
  const found = candidates.find((candidate) => fs.existsSync(candidate))
  if (!found) {
    throw new Error(`PluginHost.vue not found; tried: ${candidates.join(', ')}`)
  }
  return found
}

/** 与既有域6 组件测试相同的辅助块加载方式（纯函数，无 DOM 依赖）。 */
function loadPluginHostHelpers() {
  const source = fs.readFileSync(resolvePluginHostSource(), 'utf8')
  const match = source.match(/<script>([\s\S]*?)<\/script>/)
  if (!match) throw new Error('PluginHost.vue should expose testable helpers')

  const moduleCode = match[1]
    .replaceAll('export const ', 'const ')
    .replaceAll('export function ', 'function ')

  const sandbox = { setTimeout, clearTimeout }
  vm.runInNewContext(`${moduleCode}
Object.assign(globalThis, {
  createPluginBridgeHandshakeToken,
  isPluginBridgeHandshakeValid,
  pluginIframeTargetOrigin,
  PLUGIN_SLOT_SANITIZE_OPTIONS,
})`, sandbox)
  return sandbox
}

const {
  createPluginBridgeHandshakeToken,
  isPluginBridgeHandshakeValid,
  pluginIframeTargetOrigin,
  PLUGIN_SLOT_SANITIZE_OPTIONS,
} = loadPluginHostHelpers()

describe('PluginHost handshake helpers', () => {
  it('mints unique non-empty tokens', () => {
    const tokens = new Set()
    for (let index = 0; index < 20; index += 1) tokens.add(createPluginBridgeHandshakeToken())
    expect(tokens.size).toBe(20)
    for (const token of tokens) {
      expect(token).toMatch(/^sfh_[a-z0-9]+_[a-z0-9]+$/)
    }
  })

  it('fails closed on empty or mismatched tokens', () => {
    const token = createPluginBridgeHandshakeToken()
    expect(isPluginBridgeHandshakeValid(token, token)).toBe(true)
    expect(isPluginBridgeHandshakeValid(token, 'sfh_other')).toBe(false)
    expect(isPluginBridgeHandshakeValid('', '')).toBe(false)
    expect(isPluginBridgeHandshakeValid('', undefined)).toBe(false)
    expect(isPluginBridgeHandshakeValid(token, '')).toBe(false)
  })

  it('pins the push origin only for shell-origin documents', () => {
    const shellOrigin = 'http://storyforge-shell.localhost'
    const token = 'b'.repeat(64)
    expect(pluginIframeTargetOrigin(`${shellOrigin}/${token}`, shellOrigin)).toBe(shellOrigin)
    // blob: 兜底（非 Tauri）与空 src 保持 '*'
    expect(pluginIframeTargetOrigin('blob:http://localhost/abc', shellOrigin)).toBe('*')
    expect(pluginIframeTargetOrigin('', shellOrigin)).toBe('*')
    expect(pluginIframeTargetOrigin(`${shellOrigin}/${token}`, '')).toBe('*')
    // 后缀拼接不构成壳源（防 `http://storyforge-shell.localhost.evil.com/`）
    expect(pluginIframeTargetOrigin(`http://storyforge-shell.localhost.evil.com/${token}`, shellOrigin)).toBe('*')
  })

  it('forbids <style> in the slot sanitize config (M-31a)', () => {
    expect(PLUGIN_SLOT_SANITIZE_OPTIONS.FORBID_TAGS).toEqual(['style'])
  })
})

const registerDocCalls = []
const TOKEN = 'a'.repeat(64)
const SHELL_ORIGIN = 'http://storyforge-shell.localhost'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: async (command, payload) => {
    if (command === 'card_shell_register_doc') {
      registerDocCalls.push(payload?.html || '')
      return TOKEN
    }
    if (command === 'card_shell_unregister_doc') return true
    return null
  },
}))

// 无 id 前缀 ⇒ Vue 用文件名推导出 <plugin-host>。
let PluginHost = null

const PLUGIN = {
  id: 'plugin-m24',
  name: 'M24 fixture',
  entry_html: '<div id="app">plugin</div>',
  permissions: [],
}

function mountHost() {
  return mount(PluginHost, {
    props: { plugin: PLUGIN },
    attachTo: document.body,
  })
}

function frameWindowOf(wrapper) {
  const iframe = wrapper.element.querySelector('iframe')
  expect(iframe).toBeTruthy()
  return iframe.contentWindow
}

function handshakeTokenFromLastDoc() {
  const doc = registerDocCalls.at(-1) || ''
  // 桥脚本模板是 `handshake: "..."`（冒号后有空格），别用无空格的形状。
  const match = doc.match(/handshake:\s*"(sfh_[^"]*)"/)
  return match ? match[1] : null
}

function postFrom(_wrapper, source, data) {
  window.dispatchEvent(new MessageEvent('message', { data, source }))
}

/**
 * happy-dom 不保证 iframe 的 load 事件在同 tick 内派发，而真实浏览器里桥脚本的
 * `sf:ready` 必然发生在 load 之后。这里显式触发一次 load，让「握手前」的断言不
 * 依赖宿主实现细节（onIframeLoad 只做撤销信任，不会自己放行）。
 */
async function settleFrameLoad(wrapper) {
  await wrapper.find('iframe').trigger('load')
  await flushPromises()
}

describe('PluginHost bridge handshake (M-24)', () => {
  beforeEach(async () => {
    registerDocCalls.length = 0
    if (!PluginHost) {
      PluginHost = (await import('../../src/components/PluginHost.vue')).default
    }
  })

  it('embeds a fresh per-document handshake token and pins the push origin', async () => {
    const wrapper = mountHost()
    await flushPromises()

    const doc = registerDocCalls.at(-1)
    expect(doc).toBeTruthy()
    expect(doc).toMatch(/handshake:\s*"sfh_[a-z0-9]+_[a-z0-9]+"/)
    // 推送 origin 钉死到壳源（blob: 兜底才用 '*'）。
    expect(wrapper.element.querySelector('iframe').getAttribute('src'))
      .toBe(`${SHELL_ORIGIN}/${TOKEN}`)

    wrapper.unmount()
  })

  it('registers exactly once per source change and embeds the token it later accepts (M-24 invariant)', async () => {
    const wrapper = mountHost()
    await flushPromises()
    // 初挂载：恰好注册一次（旧实现会因令牌↔文档自激而注册多次）。
    expect(registerDocCalls.length).toBe(1)
    const firstToken = handshakeTokenFromLastDoc()
    expect(firstToken).toBeTruthy()

    // 源变化 ⇒ 恰好再注册一次，且新文档带**新**令牌。
    await wrapper.setProps({ plugin: { ...PLUGIN, entry_html: '<div id="v2">v2</div>' } })
    await flushPromises()
    expect(registerDocCalls.length).toBe(2)
    const secondToken = handshakeTokenFromLastDoc()
    expect(secondToken).toBeTruthy()
    expect(secondToken).not.toBe(firstToken)

    // 不变量：注册文档里的令牌 == 随后被接受的令牌；旧令牌必须失效。
    const frame = frameWindowOf(wrapper)
    await settleFrameLoad(wrapper)
    postFrom(wrapper, frame, { type: 'sf:ready', pluginId: PLUGIN.id, handshake: firstToken })
    await flushPromises()
    postFrom(wrapper, frame, {
      type: 'sf:ui:mount',
      pluginId: PLUGIN.id,
      slot: 'sidebar',
      html: '<p>stale-token</p>',
    })
    await flushPromises()
    expect(wrapper.find('.plugin-slot-content').exists()).toBe(false)

    postFrom(wrapper, frame, { type: 'sf:ready', pluginId: PLUGIN.id, handshake: secondToken })
    await flushPromises()
    postFrom(wrapper, frame, {
      type: 'sf:ui:mount',
      pluginId: PLUGIN.id,
      slot: 'sidebar',
      html: '<p>fresh-token</p>',
    })
    await flushPromises()
    expect(wrapper.find('.plugin-slot-content').text()).toContain('fresh-token')

    wrapper.unmount()
  })

  it('ignores every message until the current document completes the handshake', async () => {
    const wrapper = mountHost()
    await flushPromises()
    const frame = frameWindowOf(wrapper)
    await settleFrameLoad(wrapper)
    const token = handshakeTokenFromLastDoc()
    expect(token).toBeTruthy()

    // 1) 未握手（文档已 load）：即使来源窗口正确 + 令牌正确也不放行——令牌只验证
    //    握手消息本身，其它消息必须先有已完成的握手。
    postFrom(wrapper, frame, {
      type: 'sf:ui:mount',
      pluginId: PLUGIN.id,
      slot: 'sidebar',
      html: '<p>pre-handshake</p>',
    })
    await flushPromises()
    expect(wrapper.find('.plugin-slot-content').exists()).toBe(false)

    // 2) 错误令牌不能完成握手（含空令牌）。
    postFrom(wrapper, frame, { type: 'sf:ready', pluginId: PLUGIN.id, handshake: '' })
    postFrom(wrapper, frame, { type: 'sf:ready', pluginId: PLUGIN.id, handshake: 'sfh_forged' })
    await flushPromises()
    postFrom(wrapper, frame, {
      type: 'sf:ui:mount',
      pluginId: PLUGIN.id,
      slot: 'sidebar',
      html: '<p>forged-token</p>',
    })
    await flushPromises()
    expect(wrapper.find('.plugin-slot-content').exists()).toBe(false)

    // 3) 正确令牌完成握手 → 放行；同时 ready 事件只在此时才发。
    postFrom(wrapper, frame, { type: 'sf:ready', pluginId: PLUGIN.id, handshake: token })
    await flushPromises()
    expect(wrapper.emitted('ready')).toBeTruthy()

    postFrom(wrapper, frame, {
      type: 'sf:ui:mount',
      pluginId: PLUGIN.id,
      slot: 'sidebar',
      html: '<p>after-handshake</p>',
    })
    await flushPromises()
    expect(wrapper.find('.plugin-slot-content').text()).toContain('after-handshake')

    wrapper.unmount()
  })

  it('revokes trust on frame self-navigation and never trusts a WindowProxy again', async () => {
    const wrapper = mountHost()
    await flushPromises()
    const frame = frameWindowOf(wrapper)
    await settleFrameLoad(wrapper)
    const token = handshakeTokenFromLastDoc()

    postFrom(wrapper, frame, { type: 'sf:ready', pluginId: PLUGIN.id, handshake: token })
    await flushPromises()
    postFrom(wrapper, frame, {
      type: 'sf:ui:mount',
      pluginId: PLUGIN.id,
      slot: 'sidebar',
      html: '<p>trusted</p>',
    })
    await flushPromises()
    expect(wrapper.find('.plugin-slot-content').text()).toContain('trusted')

    // 帧自我导航：load 再次触发而 WindowProxy 不变。旧文档的令牌随之作废。
    await wrapper.find('iframe').trigger('load')
    await flushPromises()

    // 新文档拿不到令牌，用旧令牌重放也不被接受。
    postFrom(wrapper, frame, { type: 'sf:ready', pluginId: PLUGIN.id, handshake: token })
    await flushPromises()
    postFrom(wrapper, frame, {
      type: 'sf:ui:mount',
      pluginId: PLUGIN.id,
      slot: 'statusbar',
      html: '<p>after-navigation</p>',
    })
    await flushPromises()
    expect(wrapper.find('[data-plugin-slot="statusbar"]').exists()).toBe(false)

    wrapper.unmount()
  })

  it('rejects bridge messages whose source window is not this iframe', async () => {
    const wrapper = mountHost()
    await flushPromises()
    const token = handshakeTokenFromLastDoc()

    postFrom(wrapper, { name: 'rogue-frame' }, {
      type: 'sf:ready',
      pluginId: PLUGIN.id,
      handshake: token,
    })
    await flushPromises()
    postFrom(wrapper, { name: 'rogue-frame' }, {
      type: 'sf:ui:mount',
      pluginId: PLUGIN.id,
      slot: 'sidebar',
      html: '<p>rogue</p>',
    })
    await flushPromises()
    expect(wrapper.find('.plugin-slot-content').exists()).toBe(false)

    wrapper.unmount()
  })

  it('strips <style> from slot HTML so a plugin cannot inject host-global CSS (M-31a)', async () => {
    const wrapper = mountHost()
    await flushPromises()
    const frame = frameWindowOf(wrapper)
    await settleFrameLoad(wrapper)
    const token = handshakeTokenFromLastDoc()
    postFrom(wrapper, frame, { type: 'sf:ready', pluginId: PLUGIN.id, handshake: token })
    await flushPromises()

    postFrom(wrapper, frame, {
      type: 'sf:ui:mount',
      pluginId: PLUGIN.id,
      slot: 'sidebar',
      html: '<style>body{display:none}</style><button class="sf-x" style="color:red">ok</button>',
    })
    await flushPromises()

    const slot = wrapper.find('.plugin-slot-content')
    expect(slot.exists()).toBe(true)
    expect(slot.html()).not.toContain('<style')
    expect(slot.html()).not.toContain('display:none')
    // 正常插件 UI 与行内样式保留，渲染不受影响。
    //
    // 诚实边界（Lead 收口实测）：happy-dom 环境下 DOMPurify 3.4.12 的默认白名单
    // 解析为**空**——`sanitize('<p>hi</p>') === 'hi'`、`sanitize('<span>hi</span>') ===
    // 'hi'，任何元素都会被"去标签留文本"（`isSupported === true` 也不代表白名单可用）。
    // 因此"元素与行内 style 属性保留"这条控制组**不能**打在 DOM 结果上（那测的是
    // happy-dom 的白名单，不是我们的逻辑）。改为：DOM 侧断言样式与 CSS 正文确实
    // 消失（本用例要防的注入面），控制组断言打在我们的预处理函数上（环境无关，
    // 且是真实断言而非恒真）。
    const { stripStyleElementsFromSlotHtml: stripStyles } =
      await import('../../src/components/PluginHost.vue')
    const rawSlotHtml = '<style>body{display:none}</style>'
      + '<button class="sf-x" style="color:red">ok</button>'
    expect(stripStyles(rawSlotHtml))
      .toBe('<button class="sf-x" style="color:red">ok</button>')
    // 无 `<style` 时输入逐字节不变（不引入额外改写）。
    const untouched = '<button class="sf-x" style="color:red">ok</button>'
    expect(stripStyles(untouched)).toBe(untouched)
    // N-R4-02：未闭合的 `<style>`（HTML 里 `<style/>` 等价于开标签）其后按浏览器
    // 语义全是 CSS 文本 ⇒ 截断，否则 CSS 正文会以裸文本留在宿主 DOM。
    expect(stripStyles('<style/>body{display:none}')).toBe('')
    expect(stripStyles('<style>body{display:none}')).toBe('')
    expect(stripStyles('<p>hi</p><style>a{b:c}')).toBe('<p>hi</p>')
    expect(slot.text()).toContain('ok')

    wrapper.unmount()
  })
})
