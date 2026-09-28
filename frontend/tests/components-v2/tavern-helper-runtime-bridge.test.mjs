import { describe, it, expect, beforeEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'

/**
 * M-07：TavernHelperRuntime 的 postMessage 桥必须校验 event.source。
 *
 * 监听器挂在 window 上，旧实现只认 `d.__sf_th_bridge` 标记字段，因此任何能向
 * 主窗 postMessage 的 frame（含加载远程卡 HTML 的 CardShellHost）都能伪造桥
 * 请求。现在按 mvu-runtime-bridge.js:1-4 的基线做帧归属校验。
 *
 * 判定「处理器是否受理」的可见副产物：一条合法 bootstrap 走 fetch_text 分支，
 * 在测试环境（无 Tauri）由 `cardShellFetchUrl` 抛出
 * `card shell fetch requires Tauri host`（tauri-api.js:1217-1222），被 reply
 * 写进 lastError 并渲染到状态条。若帧归属校验把消息丢掉，这段文本不会出现
 * ——无需断言组件内部状态。
 */

let TavernHelperRuntime = null

async function mountRuntime() {
  if (!TavernHelperRuntime) {
    TavernHelperRuntime = (await import('../../src/components/TavernHelperRuntime.vue')).default
  }
  const wrapper = mount(TavernHelperRuntime, {
    props: { showStatus: true, autoRun: false, shells: [] },
    attachTo: document.body,
  })
  await flushPromises()
  return wrapper
}

function frameWindowOf(wrapper) {
  const iframe = wrapper.element.querySelector('iframe')
  expect(iframe).toBeTruthy()
  return iframe.contentWindow
}

function postFrom(source, data) {
  window.dispatchEvent(new MessageEvent('message', { data, source }))
}

// 受理副产物（task-29 ②）：宿主对**请求方帧**的出站回复
// `ev.source.postMessage({ __sf_th_bridge_res: <id>, result, error })`
// （TavernHelperRuntime.vue:749-755）。它只在请求被受理后才出现，因此能真正
// 区分「受理 / 未受理」；状态条纹案（'TavernHelper无脚本重新执行'）在空态本来
// 就会出现，用它当标记是假阳性（前一轮的错误）。
// 探针双通道：可控 window-like 对象直接替换 postMessage 记录；真实帧则尝试替换、
// 失败（跨源 WindowProxy 不可写）则退化为监听该帧收到的 message 事件。
function captureReplies(windowLike) {
  const replies = []
  try {
    const original = windowLike.postMessage?.bind(windowLike)
    windowLike.postMessage = (message, targetOrigin) => {
      replies.push(message)
      if (original) original(message, targetOrigin)
    }
    return replies
  } catch (_) {
    windowLike.addEventListener('message', (event) => replies.push(event.data))
    return replies
  }
}

const repliedToRequest = (replies, id) =>
  replies.some((message) => message && message.__sf_th_bridge_res === id)

const FORGED_PROBE = {
  __sf_th_bridge: true,
  id: 'forged-1',
  type: 'fetch_text',
  payload: { url: 'file:///etc/passwd' },
}

describe('TavernHelperRuntime bridge source check (M-07)', () => {
  beforeEach(() => {
    document.body.innerHTML = ''
  })

  it('drops forged bridge messages from a window outside the TH frame chain', async () => {
    const wrapper = await mountRuntime()
    expect(frameWindowOf(wrapper)).toBeTruthy()

    // 伪造帧：任意能向主窗 postMessage 的窗口（CardShellHost / 弹窗 / 同级 frame）。
    const rogue = { name: 'rogue-frame' }
    const rogueReplies = captureReplies(rogue)
    postFrom(rogue, FORGED_PROBE)
    await flushPromises()

    // 伪造帧不得收到任何受理回复（受理的唯一可观测副产物，见文件头注释）。
    expect(repliedToRequest(rogueReplies, FORGED_PROBE.id)).toBe(false)
    expect(rogueReplies).toEqual([])
    expect(wrapper.text()).not.toContain('passwd')

    wrapper.unmount()
  })

  it('drops forged messages with no source at all (synthetic events)', async () => {
    const wrapper = await mountRuntime()
    // 监听真实帧：若无 source 的消息被误受理，宿主会往自己的帧回消息。
    const frameReplies = captureReplies(frameWindowOf(wrapper))

    postFrom(null, FORGED_PROBE)
    await flushPromises()

    expect(repliedToRequest(frameReplies, FORGED_PROBE.id)).toBe(false)

    wrapper.unmount()
  })

  it('still accepts bridge messages from its own iframe', async () => {
    const wrapper = await mountRuntime()
    const frame = frameWindowOf(wrapper)
    const frameReplies = captureReplies(frame)

    postFrom(frame, FORGED_PROBE)
    await flushPromises()

    // 受理 ⇒ 宿主对请求方帧回出 `__sf_th_bridge_res: forged-1`
    // （fetch_text 在无 Tauri 下失败，回复里带 error 也算受理）。
    expect(repliedToRequest(frameReplies, FORGED_PROBE.id)).toBe(true)

    wrapper.unmount()
  })

  it('accepts messages from a nested frame inside the TH shell (parent chain)', async () => {
    const wrapper = await mountRuntime()
    const frame = frameWindowOf(wrapper)

    // 壳内嵌套 iframe：event.source 是内层窗口，沿 parent 链可归属本壳。
    const nested = { parent: frame }
    const nestedReplies = captureReplies(nested)
    postFrom(nested, FORGED_PROBE)
    await flushPromises()

    expect(repliedToRequest(nestedReplies, FORGED_PROBE.id)).toBe(true)

    wrapper.unmount()
  })
})
