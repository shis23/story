import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import RichContent from '../../src/components-v2/st/RichContent.vue'

// 注：happy-dom 下 DOMPurify 的清洗结果与浏览器不一致（会丢 <details>、留 on*），
// 这里只断言分段结构；清洗本身由 DOMPurify 在真实浏览器里保证。
describe('RichContent raw card HTML blocks', () => {
  it('renders a complete status bar block as html and keeps prose escaped', () => {
    const content = '我推开门，<b>没</b>说话。\n'
      + '<details><summary>状态栏</summary><div>时间：上午</div></details>'
    const wrapper = mount(RichContent, { props: { content, sourceContent: content } })

    const blocks = wrapper.findAll('[data-testid="rich-raw-html"]')
    expect(blocks).toHaveLength(1)
    expect(blocks[0].find('summary').text()).toBe('状态栏')
    expect(blocks[0].text()).toContain('时间：上午')
    // 块外的正文仍是转义文本
    expect(wrapper.html()).toContain('&lt;b&gt;没&lt;/b&gt;')
  })

  it('keeps unclosed or unknown tags on the escaped text path', () => {
    const content = '正文 <div class="bar">还没闭合 <data_block>hp=5</data_block>'
    const wrapper = mount(RichContent, { props: { content, sourceContent: content } })
    expect(wrapper.find('[data-testid="rich-raw-html"]').exists()).toBe(false)
    expect(wrapper.html()).toContain('&lt;div class="bar"&gt;')
  })
})
