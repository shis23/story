// 纯函数：卡名解析、文件名清洗、会话标题截断、开场白 HTML 转文字、底层模型错误提示。
// 运行：node --test integrations/dsh-storyforge/test/*.test.mjs

import assert from 'node:assert/strict'
import { test } from 'node:test'

import { nextFreeName, readCardName, sanitizeCardFileName } from '../lib/cards.js'
import { fitSessionTitle, friendlyError, llmHint, readableText } from '../lib/text.js'
import { pngCard } from './support/png.mjs'

test('readCardName reads V3 before V2 from PNG text chunks and V1/V2 from JSON', () => {
  const both = pngCard({ chara: { data: { name: '旧名' } }, ccv3: { spec: 'chara_card_v3', data: { name: '莫挽舟' } } })
  assert.equal(readCardName(both, '.png'), '莫挽舟')
  assert.equal(readCardName(pngCard({ chara: { name: 'Seraphina' } }), '.PNG'), 'Seraphina')
  assert.equal(readCardName(pngCard({ Comment: { name: 'x' } }), '.png'), null)
  assert.equal(readCardName(Buffer.from('not a png'), '.png'), null)

  const v2 = Buffer.from(`\uFEFF${JSON.stringify({ spec: 'chara_card_v2', data: { name: ' 林间 ' } })}`)
  assert.equal(readCardName(v2, '.json'), '林间')
  assert.equal(readCardName(Buffer.from(JSON.stringify({ name: 'V1' })), '.json'), 'V1')
  assert.equal(readCardName(Buffer.from('[1,2]'), '.json'), null)
  assert.equal(readCardName(Buffer.from('{broken'), '.json'), null)
  assert.equal(readCardName(Buffer.from('{}'), '.txt'), null)
})

test('sanitizeCardFileName keeps only a safe last segment with a card extension', () => {
  assert.equal(sanitizeCardFileName('C:\\Users\\me\\Downloads\\莫挽舟.png'), '莫挽舟.png')
  assert.equal(sanitizeCardFileName('../../etc/passwd.json'), 'passwd.json')
  assert.equal(sanitizeCardFileName('a<b>:c|d?.JSON'), 'abcd.json')
  assert.equal(sanitizeCardFileName('CON.png'), 'CON_.png')
  assert.equal(sanitizeCardFileName('...png'), 'card.png')
  assert.equal(sanitizeCardFileName(' .png'), 'card.png')
  assert.equal(sanitizeCardFileName('card.exe'), null)
  assert.equal(sanitizeCardFileName(undefined), null)
  assert.equal(Array.from(sanitizeCardFileName(`${'长'.repeat(200)}.png`)).length, 84)
})

test('nextFreeName numbers duplicates instead of overwriting', () => {
  const taken = new Set(['a.png', 'a (2).png'])
  assert.equal(nextFreeName('b.png', (name) => taken.has(name)), 'b.png')
  assert.equal(nextFreeName('a.png', (name) => taken.has(name)), 'a (3).png')
})

test('fitSessionTitle leaves room for DSH fork suffixes within 80 bytes', () => {
  assert.equal(fitSessionTitle('  短标题 '), '短标题')
  const long = '我的假小子黑皮青梅不可能变成清冷校花，更别提她还有个美艳继母'
  const fitted = fitSessionTitle(long)
  assert.ok(fitted.endsWith('…'))
  assert.ok(Buffer.byteLength(fitted) <= 72)
  // DSH 分叉：`${title} (1)` 再截到 80 字节，括号必须完整。
  assert.ok(Buffer.byteLength(`${fitted} (1)`) <= 80)
  assert.equal(fitSessionTitle('x'.repeat(72)), 'x'.repeat(72))
  assert.equal(fitSessionTitle('x'.repeat(73)), `${'x'.repeat(69)}…`)
})

test('readableText turns status-bar HTML into readable markdown text', () => {
  const html = [
    '<p>第一段&nbsp;正文。</p>',
    '<!-- 注释 -->',
    '<style>.x{color:red}</style>',
    '<details><summary>📓 状态栏</summary>',
    '  <div style="x">',
    '    <h4>❄️ 莫挽舟</h4>',
    '    <ul>',
    '      <li><b>穿着:</b> 校服</li>',
    '',
    '      <li><b>数值:</b> 5 &amp; 20</li>',
    '    </ul>',
    '  </div>',
    '</details>',
    '<StatusPlaceHolderImpl/>',
    '<user> 走进来。',
  ].join('\n')
  const text = readableText(html)
  assert.equal(
    text,
    ['第一段\u00a0正文。', '', '📓 状态栏', '', '❄️ 莫挽舟', '', '- 穿着: 校服', '- 数值: 5 & 20', '', '<user> 走进来。'].join('\n'),
  )
  assert.doesNotMatch(text, /^ {4}/m, '行首四个空格会被 Markdown 当成代码块')
  // 没有 HTML 的开场白原样返回（只去首尾空白），ST 宏不动。
  assert.equal(readableText('  <USER> 推开门。\n\n　　她抬起头。 '), '<USER> 推开门。\n\n　　她抬起头。')
  assert.equal(readableText(undefined), '')
})

test('llmHint and friendlyError explain known model failures', () => {
  const raw = 'LLM error: HTTP 请求失败: MISSING_CREDENTIAL: llm-deepseek: no API key for provider route "deepseek-official"'
  assert.match(llmHint(raw), /还没有配置 API Key/)
  assert.match(llmHint('INVALID_CREDENTIAL: bad'), /无效/)
  assert.match(llmHint('HTTP 429 Too Many Requests'), /限流/)
  assert.equal(llmHint('something else'), null)

  const error = Object.assign(new Error(raw), { code: 'llm' })
  const friendly = friendlyError(error)
  assert.match(friendly.message, /^底层模型还没有配置 API Key/)
  assert.ok(friendly.message.includes(raw), '原始错误保留在提示后面')
  assert.equal(friendly.code, 'llm')
  const plain = new Error('其它错误')
  assert.equal(friendlyError(plain), plain)
  assert.equal(friendlyError(friendly), friendly, '已经是提示的错误不再包一层')
})
