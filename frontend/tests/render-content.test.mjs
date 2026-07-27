import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import {
  formatContent,
  shouldRenderHtmlDisplay,
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
