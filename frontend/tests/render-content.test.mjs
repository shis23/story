import test from 'node:test'
import assert from 'node:assert/strict'
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
