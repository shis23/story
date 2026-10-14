// 卡片 HTML 块切分：与桌面实现对拍；DSH 正文去块、整条全是块时的兜底。
// 运行：node --test integrations/dsh-storyforge/test/*.test.mjs

import assert from 'node:assert/strict'
import { existsSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath, pathToFileURL } from 'node:url'

import { renderableBlocks, splitForDsh, splitRawHtmlBlocks, withoutScripts } from '../lib/html.js'

const here = dirname(fileURLToPath(import.meta.url))
const desktopFormat = resolve(here, '../../../frontend/src/utils/formatContent.js')

const STATUS = '<details open><summary>📓 状态</summary><ul><li>体力：80</li><li>好感：35</li></ul></details>'

const SAMPLES = [
  '她转身离开。',
  `夜色渐深。\n\n${STATUS}`,
  `${STATUS}\n\n她推开门。\n<div class="bar"><div>内层</div></div>\n尾巴`,
  '开头<div>没闭合的块',
  '<data_block>hp=5</data_block>正文<StatusPlaceHolderImpl/>',
  '<table><tr><td>甲</td><td>乙</td></tr></table>\n中间一句\n<ul><li>x</li></ul>',
  '前\r\n\r\n<div>块</div>\r\n后',
  '<DIV>大写</DIV>',
  '<divider>不是块</divider>后面',
  '<blockquote>引<blockquote>套</blockquote></blockquote>',
  '',
]

test('splitRawHtmlBlocks matches the desktop implementation sample for sample', { skip: !existsSync(desktopFormat) && 'desktop frontend not checked out' }, async () => {
  const desktop = await import(pathToFileURL(desktopFormat).href)
  for (const sample of SAMPLES) {
    assert.deepEqual(splitRawHtmlBlocks(sample), desktop.splitRawHtmlBlocks(sample), JSON.stringify(sample))
  }
})

test('splitRawHtmlBlocks keeps unclosed blocks and data tags as text', () => {
  assert.deepEqual(splitRawHtmlBlocks('开头<div>没闭合的块'), [{ type: 'text', content: '开头<div>没闭合的块' }])
  assert.deepEqual(splitRawHtmlBlocks('<divider>x</divider>'), [{ type: 'text', content: '<divider>x</divider>' }])
  assert.deepEqual(splitRawHtmlBlocks(`夜色渐深。\n\n${STATUS}`), [
    { type: 'text', content: '夜色渐深。' },
    { type: 'html', content: STATUS },
  ])
})

test('renderableBlocks pairs every block with its readable text fallback', () => {
  assert.deepEqual(renderableBlocks('她转身离开。'), [])
  assert.deepEqual(renderableBlocks(`夜色渐深。\n${STATUS}`), [{ html: STATUS, text: '📓 状态\n\n- 体力：80\n- 好感：35' }])
})

test('splitForDsh drops rendered blocks from the body and counts them', () => {
  assert.deepEqual(splitForDsh('她转身离开。'), { body: '她转身离开。', blocks: 0 })
  assert.deepEqual(splitForDsh(`夜色渐深。\n\n${STATUS}\n\n她推开门。`), { body: '夜色渐深。\n\n她推开门。', blocks: 1 })
  // 切走块后剩下的卡壳占位也去掉（剩下的正文本身没有 HTML 迹象）。
  assert.deepEqual(splitForDsh(`她坐下。\n<StatusPlaceHolderImpl/>\n${STATUS}`), { body: '她坐下。', blocks: 1 })
  // ST 宏照旧保留。
  assert.deepEqual(splitForDsh(`<user>坐下。\n${STATUS}`), { body: '<user>坐下。', blocks: 1 })
  // 没有完整块：照旧整段转文字（未闭合的块、数据标签都留在正文里）。
  assert.deepEqual(splitForDsh('开头<div>没闭合'), { body: '开头\n没闭合', blocks: 0 })
  assert.deepEqual(splitForDsh(undefined), { body: '', blocks: 0 })
})

test('script bodies never turn into blocks or body text', () => {
  // MVU 状态栏：脚本在字符串字面量里拼 div。脚本整段去掉，只剩静态占位块。
  const script = `<script>\nfunction render(s) {\n  el.innerHTML = '<div class="row">体力 <div class="bar"><i></i></div> ' + s.体力 + '/100</div>';\n}\n</script>`
  const display = `她让开身子。\n<div id="mvu-status">加载中…</div>\n${script}`
  assert.deepEqual(splitForDsh(display), { body: '她让开身子。', blocks: 1 })
  assert.deepEqual(renderableBlocks(display), [{ html: '<div id="mvu-status">加载中…</div>', text: '加载中…' }])
  // 只有脚本、没有块：正文照旧（readableText 本来就去脚本）。
  assert.deepEqual(splitForDsh(`她坐下。\n${script}`), { body: '她坐下。', blocks: 0 })
  // 未闭合的 script 不动（与桌面切分一致）。
  assert.equal(withoutScripts('a<script>b'), 'a<script>b')
})

test('splitForDsh keeps a reply that is only blocks as text so the body is never empty', () => {
  assert.deepEqual(splitForDsh(`\n${STATUS}\n`), { body: '📓 状态\n\n- 体力：80\n- 好感：35', blocks: 0 })
  // 块之外只有空白或纯标签也算空。
  assert.deepEqual(splitForDsh(`${STATUS}\n<span> </span>`).blocks, 0)
})
