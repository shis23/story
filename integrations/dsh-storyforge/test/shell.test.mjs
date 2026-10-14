// 卡片界面（档 3）：信任判定、壳文档拼装、一次性文档路径、宿主代取、选择器变量桶。
// 运行：node --test integrations/dsh-storyforge/test/*.test.mjs

import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'

import { SHELL_ONLY_BODY, splitForDsh } from '../lib/html.js'
import {
  CARD_SHELL_MODULE_PREAMBLE,
  absolutizeModuleImports,
  buildShellDocument,
  cacheName,
  createDocRegistry,
  createShellFetcher,
  createShellVariableBuckets,
  findShellSegment,
  isTextResource,
  rewriteModuleScripts,
  shellCsp,
  shellDocResponse,
  shellLabel,
  shellTrust,
  tailParts,
} from '../lib/shell.js'

const here = dirname(fileURLToPath(import.meta.url))
const desktopHost = resolve(here, '../../../frontend/src/components/CardShellHost.vue')

const STATUS_URL = 'https://testingcf.jsdelivr.net/gh/storyforge-test/shell@1.0.0/dist/status/index.html'
const FENCED_LOAD = `\`\`\`\n<body><script>$('body').load('${STATUS_URL}')</script></body>\n\`\`\``
const INLINE_DOC = '<html><body><div id="app"></div><script>document.getElementById("app").textContent = "hi"</script></body></html>'
const STATUS_BLOCK = '<details><summary>状态</summary><ul><li>体力：80</li></ul></details>'

const MANIFEST = {
  shells: [
    { kind: 'status', source: 'remote_url', url: STATUS_URL, label: '状态栏', trigger: '/<StatusPlaceHolderImpl\\/>/' },
    { kind: 'message_html', source: 'inline_html', label: '内联', trigger: '/<sf-inline\\/>/' },
  ],
  remote_urls: ['https://files.catbox.moe/extra.html'],
  allowed_hosts: ['testingcf.jsdelivr.net', 'files.catbox.moe'],
}

//#region 信任与切分

test('shellTrust 收清单 URL 与 remote_urls，内联触发器只取 inline_html', () => {
  const trust = shellTrust(MANIFEST)
  assert.deepEqual([...trust.urls].sort(), ['https://files.catbox.moe/extra.html', STATUS_URL])
  assert.deepEqual(trust.triggers, [{ label: '内联', trigger: '/<sf-inline\\/>/' }])
  assert.deepEqual(shellTrust(null), { urls: new Set(), triggers: [] })
})

test('tailParts 按出现顺序给出 HTML 块与界面，清单内的 .load 可信、清单外的要确认', () => {
  const display = `他推门进来。\n\n${STATUS_BLOCK}\n\n${FENCED_LOAD}\n\n$('body').load('https://files.catbox.moe/other.html')`
  const parts = tailParts(display, display, shellTrust(MANIFEST))
  assert.equal(parts.length, 3)
  assert.equal(parts[0].type, 'html')
  assert.equal(parts[0].blocks.length, 1)
  assert.match(parts[0].blocks[0].html, /^<details>/)
  assert.deepEqual(parts[1], { type: 'shell', mode: 'load', kind: 'status', label: '状态栏界面', url: STATUS_URL, trusted: true })
  assert.equal(parts[2].url, 'https://files.catbox.moe/other.html')
  assert.equal(parts[2].trusted, false)
  // 界面不带正文；挂载时服务端重新取。
  assert.equal('html' in parts[1], false)
})

test('tailParts 内联文档只在原文命中卡正则时可信（模型自己写的要确认）', () => {
  const source = '旁白。\n<sf-inline/>'
  const display = `旁白。\n${INLINE_DOC}`
  const trusted = tailParts(display, source, shellTrust(MANIFEST))
  assert.equal(trusted.length, 1)
  assert.equal(trusted[0].mode, 'inline')
  assert.equal(trusted[0].trusted, true)
  assert.equal(trusted[0].start, display.indexOf('<html>'))

  const forged = tailParts(display, display, shellTrust(MANIFEST))
  assert.equal(forged[0].trusted, false)
  assert.equal(tailParts(display, source, shellTrust({ shells: [] }))[0].trusted, false)
})

test('没有界面时 tailParts 与档 1 的块一致；findShellSegment 按 URL / 起点定位', () => {
  const plain = `正文\n\n${STATUS_BLOCK}`
  assert.deepEqual(tailParts(plain, plain, shellTrust(MANIFEST)).map((part) => part.type), ['html'])
  assert.deepEqual(tailParts('只有字', '只有字', shellTrust(MANIFEST)), [])

  const display = `旁白。\n${FENCED_LOAD}\n${INLINE_DOC}`
  const load = findShellSegment(display, { mode: 'load', url: STATUS_URL })
  assert.equal(load.kind, 'status')
  const start = display.indexOf('<html>')
  const inline = findShellSegment(display, { mode: 'inline', start })
  assert.equal(inline.html, INLINE_DOC)
  assert.equal(findShellSegment(display, { mode: 'inline', start: start + 1 }), undefined)
  assert.equal(findShellSegment(display, { mode: 'load', url: 'https://files.catbox.moe/x.html' }), undefined)
  assert.equal(findShellSegment(display, null), undefined)
})

test('splitForDsh 从正文去掉界面；只有界面时用占位', () => {
  assert.deepEqual(splitForDsh(`他推门进来。\n\n${FENCED_LOAD}`), { body: '他推门进来。', blocks: 0, shells: 1 })
  assert.deepEqual(splitForDsh(FENCED_LOAD), { body: SHELL_ONLY_BODY, blocks: 0, shells: 1 })
  const mixed = splitForDsh(`${STATUS_BLOCK}\n\n${INLINE_DOC}`)
  assert.equal(mixed.shells, 1)
  assert.equal(mixed.blocks, 1)
  assert.equal(mixed.body, SHELL_ONLY_BODY)
})

test('shellLabel 认识四种界面，未知归为卡片界面', () => {
  assert.equal(shellLabel('status'), '状态栏界面')
  assert.equal(shellLabel('opening_home'), '首页界面')
  assert.equal(shellLabel('opening_custom'), '自定义开局界面')
  assert.equal(shellLabel('whatever'), '卡片界面')
})

//#endregion

//#region 壳文档

test('模块前言与桌面 CardShellHost.vue 逐字相同', () => {
  const source = readFileSync(desktopHost, 'utf8').replace(/\r\n/g, '\n')
  const block = source.match(/const CARD_SHELL_MODULE_PREAMBLE = \[\n([\s\S]*?)\n\]\.join\('\\n'\) \+ '\\n'/)
  assert.ok(block, '桌面前言的写法变了，更新这条测试')
  const lines = [...block[1].matchAll(/^\s*'((?:[^'\\]|\\.)*)',?\s*$/gm)].map((match) => match[1])
  assert.ok(lines.length > 20)
  assert.equal(CARD_SHELL_MODULE_PREAMBLE, `${lines.join('\n')}\n`)
})

test('CSP：响应头带 sandbox allow-scripts，网络指令只收白名单主机名', () => {
  const header = shellCsp(['testingcf.jsdelivr.net', 'EVIL.example/path', 'https://x.com', 'Files.Catbox.Moe', 'files.catbox.moe'], { header: true })
  assert.match(header, /^sandbox allow-scripts; default-src 'none'; /)
  assert.match(header, /script-src 'unsafe-inline' 'unsafe-eval' blob: data: https:\/\/testingcf\.jsdelivr\.net https:\/\/files\.catbox\.moe;/)
  assert.match(header, /connect-src data: blob: https:\/\/testingcf\.jsdelivr\.net https:\/\/files\.catbox\.moe;/)
  assert.doesNotMatch(header, /EVIL|x\.com/i)
  assert.match(header, /object-src 'none'/)
  assert.match(header, /form-action 'none'/)
  assert.equal(header.split('files.catbox.moe').length - 1, 6)
  const meta = shellCsp(['cdn.jsdelivr.net'])
  assert.doesNotMatch(meta, /sandbox/)
  assert.match(meta, /^default-src 'none'; /)
  assert.match(shellCsp(null), /script-src 'unsafe-inline' 'unsafe-eval' blob: data:;/)
})

test('相对 import 按页面地址改成绝对地址；没有页面地址时原样', () => {
  // 桌面的 ESM 分析器按分号切语句（卡的模块是打包产物，都带分号）。
  const source = "import a from './a.js';\nimport { b } from \"../lib/b.js\";\nimport c from 'https://cdn.jsdelivr.net/c.js';\nimport d from '/root.js';\n"
  const page = 'https://testingcf.jsdelivr.net/gh/x/y@1/dist/status/index.html'
  const out = absolutizeModuleImports(source, page)
  assert.match(out, /from 'https:\/\/testingcf\.jsdelivr\.net\/gh\/x\/y@1\/dist\/status\/a\.js'/)
  assert.match(out, /from "https:\/\/testingcf\.jsdelivr\.net\/gh\/x\/y@1\/dist\/lib\/b\.js"/)
  assert.match(out, /from 'https:\/\/cdn\.jsdelivr\.net\/c\.js'/)
  assert.match(out, /from 'https:\/\/testingcf\.jsdelivr\.net\/root\.js'/)
  assert.equal(absolutizeModuleImports(source, null), source)
})

test('内联模块脚本改成源码块 + 运行桩；带 src 的与空的原样', () => {
  const html =
    '<script type="module">import x from "./x.js"; x()</script>' +
    '<script type="module" src="./main.js"></script>' +
    '<script type="module">  </script>' +
    '<script>plain()</script>' +
    "<script type='module'>second()</script>"
  const out = rewriteModuleScripts(html, 'https://cdn.jsdelivr.net/gh/a/b/index.html')
  assert.match(out, /<script type="text\/x-storyforge-module" data-sf-module="m1">import x from "https:\/\/cdn\.jsdelivr\.net\/gh\/a\/b\/x\.js"; x\(\)<\/script>/)
  assert.match(out, /window\.__sfShellRunModule\("m1"\)/)
  assert.match(out, /data-sf-module="m2">second\(\)<\/script>/)
  assert.match(out, /<script type="module" src="\.\/main\.js"><\/script>/)
  assert.match(out, /<script type="module">  <\/script>/)
  assert.match(out, /<script>plain\(\)<\/script>/)
})

function shellDoc(html, extra = {}) {
  return buildShellDocument({
    html,
    bridgeSession: 'sfshell:test',
    pluginId: 'card-shell-test',
    allowedHosts: ['testingcf.jsdelivr.net'],
    ...extra,
  })
}

test('壳文档：注入在 head 最前，顺序为 CSP → base → ST 桥 → 宿主桥 → 兼容层 → 预加载 → 代取 → 地图兜底 → 测高', () => {
  const doc = shellDoc('<!doctype html><html><head><title>t</title></head><body><script>window.cardRan = 1</script></body></html>', {
    pageUrl: STATUS_URL,
  })
  const at = (needle) => {
    const index = doc.indexOf(needle)
    assert.ok(index >= 0, `missing ${needle}`)
    return index
  }
  const order = [
    at('<head><meta http-equiv="Content-Security-Policy"'),
    at('<base href="https://testingcf.jsdelivr.net/gh/storyforge-test/shell@1.0.0/dist/status/">'),
    at('card-shell-test'),
    at('var BRIDGE_SESSION = "sfshell:test"'),
    at('jquery/3.7.1/jquery.min.js'),
    at('window.__sfShellPreloadPromise = (async'),
    at("type: 'shell_resize'"),
    at('<title>t</title>'),
    at('window.cardRan = 1'),
  ]
  assert.deepEqual([...order].sort((a, b) => a - b), order)
  // meta CSP 不带 sandbox（meta 里无效），网络指令钉白名单。
  assert.doesNotMatch(doc.slice(order[0], order[1]), /sandbox/)
  assert.match(doc.slice(order[0], order[1]), /https:\/\/testingcf\.jsdelivr\.net/)
  assert.equal(doc.split('<head>').length - 1, 1)
})

test('壳文档：片段补齐 html/head/body；只有 body 的补 head', () => {
  const fragment = shellDoc('<div id="bar">hi</div>')
  assert.match(fragment, /^<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy"/)
  assert.match(fragment, /<body><div id="bar">hi<\/div><\/body><\/html>$/)
  assert.doesNotMatch(fragment, /<base /)

  const bodyOnly = shellDoc('<body><p>x</p></body>')
  assert.match(bodyOnly, /^<!doctype html><html><head><meta http-equiv="Content-Security-Policy"[\s\S]*<\/head><body><p>x<\/p><\/body><\/html>$/)

  const noHead = shellDoc('<html lang="zh"><body>y</body></html>')
  assert.match(noHead, /^<html lang="zh"><head><meta http-equiv/)
})

test('壳文档：嵌入的 JSON 不能闭合 script', () => {
  const doc = shellDoc('<p>x</p>', { selectorVariables: { 'local:current': { note: '</script><script>alert(1)</script>\u2028' } } })
  assert.equal(doc.includes('</script><script>alert(1)'), false)
  assert.match(doc, /\\u003c\/script\\u003e/)
  assert.equal(doc.includes('\u2028'), false)
})

//#endregion

//#region 一次性文档路径

test('文档登记：随机 token、可重复取、过期 / 超条数 / 超字节淘汰最旧的', () => {
  let clock = 0
  const registry = createDocRegistry({ maxDocs: 2, maxBytes: 10, ttlMs: 100, now: () => clock })
  const a = registry.put({ html: 'aaaa', allowedHosts: [] })
  assert.match(a, /^[A-Za-z0-9_-]{32}$/)
  assert.equal(registry.get(a).html, 'aaaa')
  assert.equal(registry.get(a).html, 'aaaa')
  assert.equal(registry.get(undefined), null)
  assert.equal(registry.get('nope'), null)

  const b = registry.put({ html: 'bbbb', allowedHosts: [] })
  const c = registry.put({ html: 'cccc', allowedHosts: [] })
  assert.equal(registry.get(a), null, '超条数淘汰最旧')
  assert.equal(registry.size, 2)

  const big = registry.put({ html: 'dddddddd', allowedHosts: [] })
  assert.equal(registry.get(b), null, '超字节淘汰最旧')
  assert.equal(registry.get(c), null)
  assert.equal(registry.get(big).html, 'dddddddd')

  clock = 101
  assert.equal(registry.get(big), null, '过期')
  assert.equal(registry.size, 0)

  const e = registry.put({ html: 'e', allowedHosts: [] })
  registry.delete(e)
  assert.equal(registry.get(e), null)
})

test('文档响应头：CSP 带 sandbox、不缓存、不带 Referer、nosniff', async () => {
  const response = shellDocResponse({ html: '<p>x</p>', allowedHosts: ['cdn.jsdelivr.net'] })
  assert.equal(response.headers.get('content-type'), 'text/html; charset=utf-8')
  assert.match(response.headers.get('content-security-policy'), /^sandbox allow-scripts; default-src 'none';.*https:\/\/cdn\.jsdelivr\.net/)
  assert.equal(response.headers.get('cache-control'), 'no-store')
  assert.equal(response.headers.get('referrer-policy'), 'no-referrer')
  assert.equal(response.headers.get('x-content-type-options'), 'nosniff')
  assert.equal(await response.text(), '<p>x</p>')
})

//#endregion

//#region 宿主代取

function withCacheDir(run) {
  const dir = mkdtempSync(join(tmpdir(), 'sf-shell-fetch-'))
  return Promise.resolve(run(dir)).finally(() => rmSync(dir, { recursive: true, force: true }))
}

function fakeFetch(routes) {
  const calls = []
  const impl = async (url, init) => {
    calls.push({ url, redirect: init?.redirect })
    const route = routes[url]
    if (!route) return new Response('missing', { status: 404 })
    return typeof route === 'function' ? route() : route.clone()
  }
  return { impl, calls }
}

test('代取：白名单内取到文本并落盘缓存，第二次走缓存', () =>
  withCacheDir(async (cacheDir) => {
    const { impl, calls } = fakeFetch({
      [STATUS_URL]: new Response('<p>status</p>', { headers: { 'content-type': 'text/html' } }),
    })
    const fetchShell = createShellFetcher({ cacheDir, allowedHosts: () => ['TestingCF.jsdelivr.net'], fetchImpl: impl })
    const first = await fetchShell(STATUS_URL)
    assert.deepEqual(first, {
      url: STATUS_URL,
      content_type: 'text/html',
      body_text: '<p>status</p>',
      body_base64: null,
      from_cache: false,
      byte_len: 13,
    })
    assert.equal(calls[0].redirect, 'manual')
    const second = await fetchShell(STATUS_URL)
    assert.equal(second.from_cache, true)
    assert.equal(second.body_text, '<p>status</p>')
    assert.equal(calls.length, 1)
    const meta = JSON.parse(readFileSync(join(cacheDir, `${cacheName(STATUS_URL)}.meta.json`), 'utf8'))
    assert.equal(meta.content_type, 'text/html')
  }))

test('代取：二进制资源回 base64', () =>
  withCacheDir(async (cacheDir) => {
    const url = 'https://files.catbox.moe/a.png'
    const { impl } = fakeFetch({ [url]: new Response(new Uint8Array([137, 80, 78, 71]), { headers: { 'content-type': 'image/png' } }) })
    const fetchShell = createShellFetcher({ cacheDir, allowedHosts: () => ['files.catbox.moe'], fetchImpl: impl })
    const result = await fetchShell(url)
    assert.equal(result.body_text, null)
    assert.equal(result.body_base64, Buffer.from([137, 80, 78, 71]).toString('base64'))
  }))

test('代取：白名单外、IP、非 http(s)、坏 URL 一律 400，不发请求', () =>
  withCacheDir(async (cacheDir) => {
    const { impl, calls } = fakeFetch({})
    const fetchShell = createShellFetcher({ cacheDir, allowedHosts: () => ['cdn.jsdelivr.net'], fetchImpl: impl })
    for (const url of ['https://evil.example/x.html', 'http://127.0.0.1/x', 'http://[::1]/x', 'file:///etc/passwd', 'not a url', undefined]) {
      await assert.rejects(fetchShell(url), (error) => error.status === 400, String(url))
    }
    assert.equal(calls.length, 0)
  }))

test('代取：重定向每一跳都复核白名单，最多 5 跳', () =>
  withCacheDir(async (cacheDir) => {
    const hop = (to) => () => new Response(null, { status: 302, headers: { location: to } })
    const { impl, calls } = fakeFetch({
      'https://cdn.jsdelivr.net/a': hop('/b'),
      'https://cdn.jsdelivr.net/b': hop('https://files.catbox.moe/c.html'),
      'https://files.catbox.moe/c.html': new Response('ok', { headers: { 'content-type': 'text/plain' } }),
      'https://cdn.jsdelivr.net/out': hop('https://evil.example/'),
      'https://cdn.jsdelivr.net/loop': hop('/loop'),
      'https://cdn.jsdelivr.net/nowhere': () => new Response(null, { status: 302 }),
      'https://cdn.jsdelivr.net/500': () => new Response('boom', { status: 500 }),
    })
    const fetchShell = createShellFetcher({ cacheDir, allowedHosts: () => ['cdn.jsdelivr.net', 'files.catbox.moe'], fetchImpl: impl })
    const followed = await fetchShell('https://cdn.jsdelivr.net/a')
    assert.equal(followed.url, 'https://cdn.jsdelivr.net/a')
    assert.equal(followed.body_text, 'ok')
    assert.deepEqual(calls.map((call) => call.url), ['https://cdn.jsdelivr.net/a', 'https://cdn.jsdelivr.net/b', 'https://files.catbox.moe/c.html'])

    await assert.rejects(fetchShell('https://cdn.jsdelivr.net/out'), (error) => error.status === 400 && /allowlist/.test(error.message))
    await assert.rejects(fetchShell('https://cdn.jsdelivr.net/loop'), /redirect limit/)
    assert.equal(calls.filter((call) => call.url === 'https://cdn.jsdelivr.net/loop').length, 6)
    await assert.rejects(fetchShell('https://cdn.jsdelivr.net/nowhere'), /no valid Location/)
    await assert.rejects(fetchShell('https://cdn.jsdelivr.net/500'), /HTTP 500/)
  }))

test('代取：超过大小上限报错且不缓存', () =>
  withCacheDir(async (cacheDir) => {
    const url = 'https://cdn.jsdelivr.net/big.js'
    const { impl } = fakeFetch({
      [url]: () => new Response('x'.repeat(20), { headers: { 'content-type': 'text/javascript' } }),
      'https://cdn.jsdelivr.net/declared.js': () =>
        new Response('x', { headers: { 'content-type': 'text/javascript', 'content-length': '999' } }),
    })
    const fetchShell = createShellFetcher({ cacheDir, allowedHosts: () => ['cdn.jsdelivr.net'], fetchImpl: impl, maxBytes: 10 })
    await assert.rejects(fetchShell(url), /too large/)
    await assert.rejects(fetchShell('https://cdn.jsdelivr.net/declared.js'), /too large: 999/)
    await assert.rejects(fetchShell(url), /too large/, '失败的结果没有进缓存')
  }))

test('文本判定与缓存文件名和桌面同口径', () => {
  assert.equal(isTextResource('text/html', 'https://x/a'), true)
  assert.equal(isTextResource('application/javascript', 'https://x/a'), true)
  assert.equal(isTextResource('image/svg+xml', 'https://x/a'), true)
  assert.equal(isTextResource('application/octet-stream', 'https://x/a.mjs'), true)
  assert.equal(isTextResource('image/png', 'https://x/a.png'), false)
  assert.match(cacheName('https://x/dir/index.html?v=1'), /^[0-9a-f]{64}_index.htmlv1$/)
  assert.match(cacheName('https://x/'), /^[0-9a-f]{64}_bin$/)
})

//#endregion

//#region 选择器变量桶

test('变量桶：按存档隔离，replace / merge / patch 与桌面同语义', () => {
  const buckets = createShellVariableBuckets()
  assert.deepEqual(buckets.get('c1'), {})
  buckets.apply('c1', { type: 'chat' }, { theme: 'dark', size: 2 }, 'replace')
  buckets.apply('c1', { type: 'chat' }, { size: 3 }, 'merge')
  assert.deepEqual(buckets.get('c1')['chat:current'], { theme: 'dark', size: 3 })
  buckets.apply('c1', { type: 'chat' }, { sets: { lang: 'zh' }, deletes: ['theme'] }, 'patch')
  assert.deepEqual(buckets.get('c1')['chat:current'], { size: 3, lang: 'zh' })
  buckets.apply('c1', { type: 'chat' }, { only: true }, undefined)
  assert.deepEqual(buckets.get('c1')['chat:current'], { only: true })
  assert.deepEqual(buckets.get('c2'), {})
})

//#endregion
