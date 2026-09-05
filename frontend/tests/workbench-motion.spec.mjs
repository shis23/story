import { expect, test } from '@playwright/test'

const fixture = '/tests/fixtures/mobile-chrome.html'

async function startSampling(page, selector) {
  await page.evaluate(selector => {
    window.motionFrames = []
    window.sampleMotion = true
    function sample() {
      const node = document.querySelector(selector)
      if (node) {
        const style = getComputedStyle(node)
        const rect = node.getBoundingClientRect()
        window.motionFrames.push({
          x: rect.x, y: rect.y, width: rect.width, height: rect.height,
          opacity: Number(style.opacity), transform: style.transform,
          rootTransform: getComputedStyle(document.querySelector('.sf-safe-screen')).transform,
        })
      }
      if (window.sampleMotion) requestAnimationFrame(sample)
    }
    requestAnimationFrame(sample)
  }, selector)
}

async function finishSampling(page) {
  await expect.poll(() => page.evaluate(() => window.motionFrames.length)).toBeGreaterThan(20)
  return page.evaluate(() => {
    window.sampleMotion = false
    return window.motionFrames
  })
}

test('sidebar slides without resizing the viewport or remounting its runtime', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 })
  await page.goto(fixture)
  await page.evaluate(() => { window.sidebarBefore = document.querySelector('aside') })
  await startSampling(page, 'aside')
  await page.getByRole('button', { name: '菜单', exact: true }).click()
  const frames = (await finishSampling(page)).filter(frame => frame.width > 0)
  expect(frames.some(frame => frame.x < -1)).toBe(true)
  expect(frames.at(-1).x).toBe(0)
  expect(frames.every(frame => frame.width === 260 && frame.rootTransform === 'none')).toBe(true)
  const sidebar = page.locator('aside')
  await sidebar.getByRole('button', { name: '关闭', exact: true }).click()
  await expect(sidebar).toHaveAttribute('inert', '')
  await expect(sidebar).toBeHidden()
  await page.setViewportSize({ width: 1280, height: 900 })
  await expect(sidebar).toBeVisible()
  expect(await sidebar.evaluate(el => el === window.sidebarBefore)).toBe(true)
})

test('desktop sidebar can be collapsed and restored without losing the draft', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 })
  await page.goto(`${fixture}?scene=writing`)
  const sidebar = page.locator('aside')
  const intent = page.getByRole('textbox', { name: '写作意图' })
  await intent.fill('侧栏收起后仍然保留')
  await page.evaluate(() => { window.sidebarBefore = document.querySelector('aside') })
  await expect(sidebar.getByRole('button', { name: '收起侧栏' })).toBeVisible({ timeout: 2000 })
  await sidebar.getByRole('button', { name: '收起侧栏' }).click()
  await expect(sidebar).toBeHidden()
  await expect(sidebar).toHaveAttribute('inert', '')
  expect((await page.locator('.story-topbar').boundingBox()).x).toBe(0)
  await page.getByRole('button', { name: '展开侧栏', exact: true }).click()
  await expect(sidebar).toBeVisible()
  await page.locator('.story-topbar').getByRole('button', { name: '收起侧栏' }).click()
  await expect(sidebar).toBeHidden()
  await page.setViewportSize({ width: 480, height: 900 })
  await page.getByRole('button', { name: '菜单', exact: true }).click()
  await expect(sidebar).toBeVisible()
  await sidebar.getByRole('button', { name: '关闭', exact: true }).click()
  await expect(sidebar).toBeHidden()
  await page.setViewportSize({ width: 1280, height: 900 })
  await expect(sidebar).toBeHidden()
  await expect(intent).toHaveValue('侧栏收起后仍然保留')
  expect(await sidebar.evaluate(el => el === window.sidebarBefore)).toBe(true)
  await page.getByRole('button', { name: '展开侧栏', exact: true }).click()
  await page.setViewportSize({ width: 480, height: 900 })
  await expect(sidebar).toBeHidden()
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
})

test('view entry and disclosure animate while the composer retains input', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 })
  await page.goto(fixture)
  await startSampling(page, '.sf-view-enter')
  await page.getByRole('button', { name: '查看写作界面' }).click()
  const frames = await finishSampling(page)
  expect(frames.some(frame => frame.opacity < 0.99)).toBe(true)
  expect(frames.at(-1).opacity).toBe(1)
  expect(frames.at(-1).transform).toBe('none')
  expect(frames.every(frame => frame.rootTransform === 'none')).toBe(true)

  const intent = page.getByRole('textbox', { name: '写作意图' })
  await intent.fill('保留这段尚未发送的写作意图')
  const disclosure = page.getByRole('button', { name: /创作过程回顾/ })
  await disclosure.scrollIntoViewIfNeeded()
  await startSampling(page, `[id="${await disclosure.getAttribute('aria-controls')}"]`)
  await disclosure.click()
  const expandedFrames = await finishSampling(page)
  expect(Math.min(...expandedFrames.map(frame => frame.height))).toBeLessThan(expandedFrames.at(-1).height)
  await expect(disclosure).toHaveAttribute('aria-expanded', 'true')
  await page.getByRole('button', { name: /正文续写/ }).click()
  await expect(page.getByText('成文 3 段 · 212 字', { exact: true })).toBeVisible()
  await disclosure.click()
  await expect(disclosure).toHaveAttribute('aria-expanded', 'false')
  await expect(intent).toHaveValue('保留这段尚未发送的写作意图')
  await page.screenshot({ path: test.info().outputPath('desktop-writing.png') })
})

test('reduced motion keeps feedback visible with no ongoing animation', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' })
  await page.goto(`${fixture}?scene=writing`)
  await page.getByRole('textbox', { name: '写作意图' }).fill('继续写作')
  await page.getByRole('button', { name: '开始写作', exact: true }).click()
  const activity = page.getByTestId('writing-activity')
  await expect(activity).toBeVisible()
  expect(await activity.evaluate(el => getComputedStyle(el, '::after').animationName)).toBe('none')
  await expect(page.locator('.story-topbar').getByRole('status')).toContainText('写作中')
  await expect.poll(() => page.evaluate(() => document.getAnimations().filter(animation =>
    animation.playState === 'running' || animation.playState === 'pending',
  ).length)).toBe(0)
  await page.getByRole('button', { name: '停止生成', exact: true }).last().click()
  await expect(activity).toBeHidden()
  await expect(page.getByRole('textbox', { name: '写作意图' })).toBeEnabled()
})

for (const width of [320, 480, 768, 1280]) {
  test(`writing layout and long titles fit at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 })
    const title = 'FixedStorySnapshotWithAnUnbrokenLongCampaignName'
    await page.goto(`${fixture}?scene=writing&title=${title}`)
    await expect(page.getByRole('heading', { name: title })).toBeVisible()
    for (const locator of [
      page.getByRole('heading', { name: title }),
      page.getByRole('textbox', { name: '写作意图' }),
      page.getByRole('button', { name: '开始写作', exact: true }),
    ]) {
      const box = await locator.boundingBox()
      expect(box.x).toBeGreaterThanOrEqual(0)
      expect(box.x + box.width).toBeLessThanOrEqual(width)
    }
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
    const titleBox = await page.getByRole('heading', { name: title }).boundingBox()
    const statusBox = await page.getByTestId('story-status').boundingBox()
    expect(titleBox.x + titleBox.width <= statusBox.x || titleBox.y + titleBox.height <= statusBox.y).toBe(true)
    await page.screenshot({ path: test.info().outputPath('writing.png'), animations: 'disabled' })
    if (width >= 1024) {
      await page.getByRole('switch', { name: '夜读模式', exact: true }).click()
      await expect(page.locator('html')).toHaveClass(/dark/)
      await page.screenshot({ path: test.info().outputPath('writing-dark.png'), animations: 'disabled' })
    }
  })
}
