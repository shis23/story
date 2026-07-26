import test from 'node:test'
import assert from 'node:assert/strict'
import {
  createCardShellFetchProxyScript,
  createCardShellMapReadyFallbackScript,
} from '../src/utils/cardShellFetchProxy.js'

test('routes binary image fetches through the host and returns a usable Response blob', async () => {
  const script = createCardShellFetchProxyScript()
  const shellWindow = {
    __sfHostFetchDataUrl: async (url) => {
      assert.equal(url, 'https://i.ibb.co/example/Maplite.webp')
      return 'data:image/webp;base64,AQIDBA=='
    },
  }

  Function('window', 'Response', 'atob', script)(shellWindow, Response, atob)
  const response = await shellWindow.fetch('https://i.ibb.co/example/Maplite.webp')
  assert.equal(response.headers.get('content-type'), 'image/webp')
  assert.deepEqual([...new Uint8Array(await response.arrayBuffer())], [1, 2, 3, 4])
})

test('streams a large shell binary from the host cache instead of expanding it into a data URL', async () => {
  const script = createCardShellFetchProxyScript()
  const nativeCalls = []
  const shellWindow = {
    fetch: async (input) => {
      nativeCalls.push(input)
      return new Response('large-map-bytes', {
        headers: { 'Content-Type': 'image/webp' },
      })
    },
    __sfHostFetchDataUrl: async (url) => {
      assert.equal(url, 'https://i.ibb.co/example/Map.webp')
      return { cacheUrl: 'storyforge-cache://localhost/example-map.webp' }
    },
  }

  Function('window', 'Response', 'atob', script)(shellWindow, Response, atob)
  const response = await shellWindow.fetch('https://i.ibb.co/example/Map.webp')

  assert.equal(await response.text(), 'large-map-bytes')
  assert.deepEqual(nativeCalls, ['storyforge-cache://localhost/example-map.webp'])
})

test('makes a slow ultra-map fallback visible instead of leaving the selected map inert', async () => {
  const script = createCardShellFetchProxyScript()
  const notices = []
  const shellDocument = {
    readyState: 'complete',
    body: { appendChild: (node) => notices.push(node) },
    querySelector: () => null,
    createElement: () => ({ style: {}, setAttribute() {}, textContent: '' }),
  }
  const shellWindow = {
    fetch: async () => new Response('standard-map'),
    __sfHostFetchDataUrl: async () => ({
      cacheUrl: 'storyforge-cache://localhost/standard-map.webp',
      fallbackMessage: '超清地图源响应过慢，已暂时显示高清地图。',
    }),
  }

  Function('window', 'Response', 'atob', 'document', script)(
    shellWindow,
    Response,
    atob,
    shellDocument,
  )
  await shellWindow.fetch('https://i.ibb.co/example/Map.webp')

  assert.equal(notices.length, 1)
  assert.match(notices[0].textContent, /超清地图源响应过慢/)
})

test('does not proxy local fetches away from their native implementation', async () => {
  const script = createCardShellFetchProxyScript()
  const nativeCalls = []
  const shellWindow = {
    fetch: async (input) => {
      nativeCalls.push(input)
      return new Response('native')
    },
    __sfHostFetchDataUrl: async () => {
      throw new Error('should not proxy local URL')
    },
  }

  Function('window', 'Response', 'atob', script)(shellWindow, Response, atob)
  const response = await shellWindow.fetch('/local/resource')
  assert.equal(await response.text(), 'native')
  assert.deepEqual(nativeCalls, ['/local/resource'])
})

test('provides an iframe-local Cache Storage shim so sandboxed map code reaches the fetch proxy', async () => {
  const script = createCardShellFetchProxyScript()
  const shellWindow = {}

  Function('window', 'Response', 'atob', script)(shellWindow, Response, atob)
  const cache = await shellWindow.caches.open('destined-map')
  await cache.put('map.webp', new Response('cached map'))
  const cached = await cache.match('map.webp')

  assert.equal(await cached.text(), 'cached map')
})

test('only suppresses a card map loading overlay after a blob-backed map image is ready', () => {
  const script = createCardShellMapReadyFallbackScript()

  assert.match(script, /\[data-page="map"\]/)
  assert.match(script, /src\.indexOf\("blob:"\)/)
  assert.match(script, /naturalWidth > 0/)
  assert.match(script, /地图加载中/)
})
