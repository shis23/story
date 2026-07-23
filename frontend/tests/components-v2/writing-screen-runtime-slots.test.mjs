import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import WritingScreen from '../../src/design/writing/WritingScreen.vue'

const StoryPageStub = {
  name: 'StoryPage',
  template: `
    <section data-testid="story-page">
      <div data-testid="story-messages">故事正文</div>
      <slot name="after-messages" />
    </section>
  `,
}

function mountWritingScreen(showOpening = true) {
  return mount(WritingScreen, {
    props: {
      title: '命定之诗',
      messages: [{ id: 'assistant-1' }],
      showOpening,
    },
    slots: {
      opening: '<section data-testid="opening-shell">完整开场</section>',
      'after-messages': '<section data-testid="runtime-status">状态栏</section>',
    },
    global: {
      stubs: {
        StoryPage: StoryPageStub,
        ProcessTimeline: true,
        ComposerBar: true,
        EmptyHero: true,
        GreetingCards: true,
      },
    },
  })
}

describe('WritingScreen runtime placement', () => {
  it('places a complete opening before the story page and status after its messages', () => {
    const wrapper = mountWritingScreen()
    const opening = wrapper.get('[data-testid="opening-shell"]')
    const story = wrapper.get('[data-testid="story-page"]')
    const status = story.get('[data-testid="runtime-status"]')

    expect(opening.element.compareDocumentPosition(story.element) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(status.element.previousElementSibling?.dataset.testid).toBe('story-messages')
  })

  it('removes the opening shell container once setup is over', () => {
    const wrapper = mountWritingScreen(false)

    expect(wrapper.find('[data-testid="opening-shell"]').exists()).toBe(false)
  })
})
