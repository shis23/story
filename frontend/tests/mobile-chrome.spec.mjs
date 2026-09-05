import { expect, test } from '@playwright/test'

const cases = [
  { width: 320, height: 740, top: 40, bottom: 24 },
  { width: 411, height: 914, top: 40, bottom: 24 },
  { width: 480, height: 800, top: 0, bottom: 0 },
  { width: 720, height: 900, top: 0, bottom: 0 },
  { width: 768, height: 1024, top: 24, bottom: 24 },
  { width: 1024, height: 900, top: 0, bottom: 0 },
  { width: 1280, height: 900, top: 0, bottom: 0 },
]

async function expectHeader(header, top) {
  await expect(header).toBeVisible()
  await expect.poll(async () => Math.round((await header.boundingBox()).y)).toBe(top)
  await expect.poll(async () => Math.round((await header.boundingBox()).height)).toBe(52)
  const box = await header.boundingBox()
  expect(box.y).toBeCloseTo(top, 0)
  expect(box.height).toBeCloseTo(52, 0)
  // Sample parent and children in one frame while the drawer is moving.
  const sample = await header.evaluate(el => ({
    header: el.getBoundingClientRect().toJSON(),
    controls: [...el.querySelectorAll('button')]
      .filter(button => button.getClientRects().length > 0)
      .map(button => button.getBoundingClientRect().toJSON()),
  }))
  for (const rect of sample.controls) {
    expect(rect.top).toBeGreaterThanOrEqual(sample.header.top)
    expect(rect.bottom).toBeLessThanOrEqual(sample.header.bottom + 1)
    expect(rect.left).toBeGreaterThanOrEqual(sample.header.left)
    expect(rect.right).toBeLessThanOrEqual(sample.header.right + 1)
  }
}

for (const viewport of cases) {
  test(`headers share one safe area at ${viewport.width}px`, async ({ page }) => {
    await page.setViewportSize(viewport)
    await page.goto('/tests/fixtures/mobile-chrome.html')
    await page.evaluate(({ top, bottom }) => {
      document.documentElement.style.setProperty('--sf-safe-top', `${top}px`)
      document.documentElement.style.setProperty('--sf-safe-bottom', `${bottom}px`)
    }, viewport)
    const topbar = page.locator('.story-topbar')
    await expectHeader(topbar, viewport.top)
    const title = await topbar.locator('[data-topbar-slot="title"]').boundingBox()
    expect(title.width).toBeGreaterThanOrEqual(100)
    for (const button of await topbar.locator('button:visible').all()) {
      const rect = await button.boundingBox()
      expect(rect.width).toBeGreaterThanOrEqual(44)
      expect(rect.height).toBeGreaterThanOrEqual(44)
      expect(title.x + title.width <= rect.x + 1 || title.x >= rect.x + rect.width - 1).toBe(true)
    }
    await page.getByRole('button', { name: '切换生成状态', exact: true }).click()
    await expect(topbar.getByRole('status')).toContainText('写作中')
    const busyTitle = await topbar.locator('[data-topbar-slot="title"]').boundingBox()
    expect(busyTitle).toEqual(title)
    await page.screenshot({ path: test.info().outputPath('topbar.png') })
    await page.getByRole('button', { name: '切换生成状态', exact: true }).click()

    if (viewport.width < 1024) {
      await page.getByRole('button', { name: '菜单', exact: true }).click()
      const sidebar = page.locator('aside')
      expect((await sidebar.boundingBox()).y).toBe(0)
      await expectHeader(sidebar.locator('[data-sidebar-header]'), viewport.top)
      await sidebar.getByRole('button', { name: '关闭', exact: true }).click()
    }

    await page.getByRole('button', { name: '查看变量', exact: true }).click()
    const dialog = page.getByRole('dialog')
    await expectHeader(dialog.locator('header'), viewport.top)
    const campaignTitle = dialog.locator('main h2')
    expect((await campaignTitle.boundingBox()).height).toBeLessThan(70)
    expect(await dialog.evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true)
    await page.screenshot({ path: test.info().outputPath('campaign.png') })
    await dialog.getByRole('button', { name: '关闭', exact: true }).click()
    await expect(dialog.locator('header')).toBeHidden()

    for (const name of ['测试功能面板', '测试旧版面板']) {
      await page.getByRole('button', { name, exact: true }).click()
      const overlay = page.getByRole('dialog')
      const close = overlay.getByRole('button', { name: '关闭', exact: true })
      const rect = await close.boundingBox()
      expect(rect.y).toBeGreaterThanOrEqual(viewport.top)
      expect(rect.y + rect.height).toBeLessThanOrEqual(viewport.top + 52)
      await close.click()
      await expect(close).toBeHidden()
    }
    await expectHeader(topbar, viewport.top)
  })
}

test('mobile campaign actions stay accessible without compressing the title', async ({ page }) => {
  await page.setViewportSize({ width: 320, height: 740 })
  await page.goto('/tests/fixtures/mobile-chrome.html')
  await page.getByRole('button', { name: '查看变量', exact: true }).click()
  const dialog = page.getByRole('dialog')
  for (const [name, event] of [
    ['导入 Bundle', 'import-bundle'],
    ['导出 ST', 'export-st'],
    ['导出 Bundle', 'export-bundle'],
  ]) {
    await dialog.getByRole('button', { name: '导入与导出', exact: true }).click()
    await page.getByRole('menuitem', { name, exact: true }).click()
    await expect(page.getByRole('menu')).toBeHidden()
    await expect(page.locator('output')).toHaveText(event)
  }
  await dialog.getByRole('button', { name: '导入与导出', exact: true }).click()
  await expect(page.getByRole('menu')).toBeFocused()
  await page.keyboard.press('Escape')
  await expect(page.getByRole('menu')).toBeHidden()
  await expect(dialog.locator('header')).toBeVisible()
  await expect(dialog.getByRole('button', { name: '导入与导出', exact: true })).toBeFocused()
})

test('full-screen header stays opaque and stationary throughout opening', async ({ page }) => {
  await page.setViewportSize({ width: 411, height: 914 })
  await page.goto('/tests/fixtures/mobile-chrome.html')
  await page.evaluate(() => {
    document.documentElement.style.setProperty('--sf-safe-top', '40px')
    window.headerFrames = []
    window.sampleHeaders = true
    function sample() {
      const header = document.querySelector('[role="dialog"] header')
      if (header) {
        let opacity = 1
        for (let node = header; node; node = node.parentElement) {
          opacity *= Number(getComputedStyle(node).opacity)
        }
        const { y, height } = header.getBoundingClientRect()
        window.headerFrames.push({ y, height, opacity })
      }
      if (window.sampleHeaders) requestAnimationFrame(sample)
    }
    requestAnimationFrame(sample)
  })
  await page.getByRole('button', { name: '查看变量', exact: true }).click()
  await expect.poll(() => page.evaluate(() => window.headerFrames.length)).toBeGreaterThan(15)
  const frames = await page.evaluate(() => {
    window.sampleHeaders = false
    return window.headerFrames
  })
  expect(frames.length).toBeGreaterThan(15)
  for (const frame of frames) {
    expect(frame.opacity).toBe(1)
    expect(frame.y).toBeCloseTo(40, 0)
    expect(frame.height).toBeCloseTo(52, 0)
  }
})
