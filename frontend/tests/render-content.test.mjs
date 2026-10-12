import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import {
  formatContent,
  shouldRenderHtmlDisplay,
  splitRawHtmlBlocks,
} from '../src/utils/formatContent.js'

test('formats markdown text while escaping raw html', () => {
  assert.equal(
    formatContent('<script>alert(1)</script>\n**bold** and *aside*'),
    '&lt;script&gt;alert(1)&lt;/script&gt;<br><strong>bold</strong> and <em>aside</em>',
  )
})

test('keeps emphasized fiction prose upright for Chinese readability', () => {
  const styles = fs.readFileSync(new URL('../src/style.css', import.meta.url), 'utf8')
  assert.match(
    styles,
    /\.prose-fiction em\s*\{[^}]*font-style:\s*normal;[^}]*opacity:\s*0\.82;/s,
  )
})

test('does not render raw ST data tags as html when display content is unchanged', () => {
  assert.equal(
    shouldRenderHtmlDisplay('<data_block>hp=5</data_block>', '<data_block>hp=5</data_block>'),
    false,
  )
})

test('renders html only for display-only derived content', () => {
  assert.equal(
    shouldRenderHtmlDisplay(
      '<div class="sf-status-bar"><span>HP 5</span></div>',
      '<data_block>hp=5</data_block>',
    ),
    true,
  )
})

test('keeps derived plain-text display content on the text formatter path', () => {
  assert.equal(shouldRenderHtmlDisplay('[HP:5] scene', '<data_block>hp=5</data_block> scene'), false)
})

test('ignores unknown custom tags in derived content', () => {
  assert.equal(shouldRenderHtmlDisplay('<status>HP 5</status>', '<data_block>hp=5</data_block>'), false)
})

test('splits a card status bar block out of raw message text', () => {
  const statusBar = '<details style="border:1px solid #ccc"><summary>状态栏</summary>'
    + '<div><ul><li><b>时间:</b> 上午</li></ul><div>嵌套</div></div></details>'
  const text = `我推开门。\r\n\r\n${statusBar}\r\n<StatusPlaceHolderImpl/>`
  assert.deepEqual(splitRawHtmlBlocks(text), [
    { type: 'text', content: '我推开门。' },
    { type: 'html', content: statusBar },
    { type: 'text', content: '<StatusPlaceHolderImpl/>' },
  ])
})

test('leaves plain text, data tags and unclosed blocks byte-identical', () => {
  for (const text of [
    '普通正文\n第二行',
    '<data_block>hp=5</data_block>',
    '正文 <div class="bar">流式中还没闭合',
    '<span>行内</span>不算块',
  ]) {
    assert.deepEqual(splitRawHtmlBlocks(text), [{ type: 'text', content: text }])
  }
})
