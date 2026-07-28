import test from 'node:test'
import assert from 'node:assert/strict'
import {
  SHELL_CACHE_ORIGINS,
  SHELL_MODULE_ORIGINS,
  buildShellCspContent,
  buildShellCspMetaTag,
} from '../src/utils/cardShellCsp.js'

test('pins every network-capable directive to the allowlist hosts', () => {
  const csp = buildShellCspContent(['files.catbox.moe', 'i.ibb.co'])
  const directives = Object.fromEntries(
    csp.split(';').map((d) => {
      const [name, ...sources] = d.trim().split(/\s+/)
      return [name, sources]
    }),
  )

  assert.deepEqual(directives['default-src'], ["'none'"])
  for (const name of ['img-src', 'script-src', 'connect-src', 'font-src', 'media-src', 'style-src']) {
    assert.ok(
      directives[name].includes('https://files.catbox.moe'),
      `${name} must include allowlisted host`,
    )
    assert.ok(
      !directives[name].some((s) => s === 'https:' || s === '*'),
      `${name} must not allow arbitrary hosts`,
    )
  }
  // 本地缓存协议源（大资源走 storyforge-cache）必须可用
  for (const origin of SHELL_CACHE_ORIGINS) {
    assert.ok(directives['img-src'].includes(origin))
    assert.ok(directives['media-src'].includes(origin))
  }
  // 模块运行器依赖 blob:，宿主代持二进制走 data:
  assert.ok(directives['script-src'].includes('blob:'))
  for (const origin of SHELL_MODULE_ORIGINS) {
    assert.ok(directives['script-src'].includes(origin))
    assert.ok(directives['connect-src'].includes(origin))
  }
  assert.ok(directives['img-src'].includes('data:'))
  assert.deepEqual(directives['object-src'], ["'none'"])
  assert.deepEqual(directives['form-action'], ["'none'"])
})

test('empty allowlist fails closed to local-only sources', () => {
  const csp = buildShellCspContent([])
  assert.ok(!csp.includes('https://'), 'no network host may appear')
  assert.ok(csp.includes("default-src 'none'"))
  assert.ok(csp.includes('blob:'))
})

test('rejects malformed allowlist entries that could smuggle CSP sources', () => {
  const csp = buildShellCspContent([
    'evil.com; script-src *',
    'https://scheme-injected.com',
    'spaced host.com',
    '*',
    '',
    null,
    'ok-host.example',
  ])
  assert.ok(csp.includes('https://ok-host.example'))
  assert.ok(!csp.includes('evil.com'))
  assert.ok(!csp.includes('scheme-injected'))
  assert.ok(!csp.includes(' * '))
})

test('meta tag wraps the policy and escapes quotes', () => {
  const tag = buildShellCspMetaTag(['files.catbox.moe'])
  assert.ok(tag.startsWith('<meta http-equiv="Content-Security-Policy" content="'))
  assert.ok(tag.endsWith('">'))
  assert.ok(tag.includes('https://files.catbox.moe'))
  // 政策本体不得包含裸双引号（'none' 用单引号），避免提前闭合属性
  const content = tag.slice(tag.indexOf('content="') + 9, -2)
  assert.ok(!content.includes('"'))
})
