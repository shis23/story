import { expect, test } from '@playwright/test'

const fixture = '/tests/fixtures/mobile-chrome.html?scene=writing'
const palettes = [
  { id: 'teal', label: '青绿' },
  { id: 'blue', label: '蓝灰' },
  { id: 'rose', label: '玫红' },
  { id: 'classic', label: '经典' },
]

async function waitForAppearance(page) {
  await expect.poll(() => page.evaluate(() => document.getAnimations().filter(animation =>
    animation.playState === 'running' || animation.playState === 'pending',
  ).length)).toBe(0)
}

test('palette selection preserves the draft, supports keyboard input and survives reload', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 })
  await page.goto(fixture)
  const intent = page.getByRole('textbox', { name: '写作意图' })
  await intent.fill('配色改变，故事继续')
  await page.getByRole('radio', { name: '玫红', exact: true }).check()
  await expect(page.locator('html')).toHaveAttribute('data-palette', 'rose')
  await page.getByRole('switch', { name: '夜读模式' }).click()
  await expect(page.locator('html')).toHaveClass(/dark/)
  await expect(intent).toHaveValue('配色改变，故事继续')
  await expect(page.locator('aside')).toBeVisible()
  await page.reload()
  await expect(page.getByRole('radio', { name: '玫红', exact: true })).toBeChecked()
  await expect(page.getByRole('switch', { name: '夜读模式' })).toBeChecked()
  await page.getByRole('radio', { name: '玫红', exact: true }).focus()
  await page.keyboard.press('ArrowLeft')
  await expect(page.getByRole('radio', { name: '蓝灰', exact: true })).toBeChecked()
  await expect(page.locator('html')).toHaveAttribute('data-palette', 'blue')
  await page.getByRole('switch', { name: '夜读模式' }).click()
  await expect(page.locator('html')).not.toHaveClass(/dark/)
  await expect(page.getByRole('radio', { name: '蓝灰', exact: true })).toBeChecked()
})

test('classic selection survives reload and releases its colors when another palette is chosen', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 })
  await page.goto(fixture)
  await page.getByRole('radio', { name: '经典', exact: true }).check()
  await page.getByRole('switch', { name: '夜读模式' }).click()
  await page.reload()
  await expect(page.getByRole('radio', { name: '经典', exact: true })).toBeChecked()
  await expect(page.getByRole('switch', { name: '夜读模式' })).toBeChecked()
  await page.getByRole('radio', { name: '青绿', exact: true }).check()
  await waitForAppearance(page)
  const restored = await page.evaluate(() => ({
    background: getComputedStyle(document.documentElement).getPropertyValue('--color-bg').trim(),
    foreground: getComputedStyle(document.querySelector('aside button.bg-accent')).color,
  }))
  expect(restored).toEqual({ background: '#16191d', foreground: 'rgb(255, 255, 255)' })
})

test('palette controls fit short mobile windows and never close the drawer', async ({ page }) => {
  await page.setViewportSize({ width: 320, height: 480 })
  await page.goto(fixture)
  await page.getByRole('button', { name: '菜单', exact: true }).click()
  const picker = page.getByRole('group', { name: '配色', exact: true })
  await expect(picker).toBeVisible()
  for (const { label } of palettes) {
    const radio = page.getByRole('radio', { name: label, exact: true })
    await radio.check()
    await expect(page.locator('aside')).toBeVisible()
    const box = await radio.locator('..').boundingBox()
    expect(box.width).toBeGreaterThanOrEqual(44)
    expect(box.height).toBeGreaterThanOrEqual(44)
    expect(box.y).toBeGreaterThanOrEqual(0)
    expect(box.y + box.height).toBeLessThanOrEqual(480)
  }
  await page.getByRole('switch', { name: '夜读模式' }).click()
  await expect(page.locator('html')).toHaveClass(/dark/)
  await waitForAppearance(page)
  await page.screenshot({ path: test.info().outputPath('mobile-classic-night.png') })
  await page.locator('aside').getByRole('button', { name: '关闭', exact: true }).click()
  await expect(page.locator('aside')).toBeHidden()
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
})

for (const { id, label } of palettes) {
  for (const mode of ['light', 'dark']) {
    test(`${id} ${mode} colors apply across surfaces with readable text`, async ({ page }) => {
      await page.setViewportSize({ width: 1280, height: 900 })
      await page.goto(fixture)
      await page.getByRole('radio', { name: label, exact: true }).check()
      if (mode === 'dark') await page.getByRole('switch', { name: '夜读模式' }).click()
      await expect(page.locator('html')).toHaveAttribute('data-palette', id)
      await waitForAppearance(page)
      const appearance = await page.evaluate(() => {
        const root = document.documentElement
        const token = name => getComputedStyle(root).getPropertyValue(name).trim()
        const rgb = color => {
          const probe = document.createElement('span')
          probe.style.color = color
          document.body.append(probe)
          const channels = getComputedStyle(probe).color.match(/[\d.]+/g).slice(0, 3).map(Number)
          probe.remove()
          return channels
        }
        const luminance = color => rgb(color).map(value => {
          const channel = value / 255
          return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4
        }).reduce((sum, value, index) => sum + value * [0.2126, 0.7152, 0.0722][index], 0)
        const contrast = (a, b) => {
          const [lighter, darker] = [luminance(a), luminance(b)].sort((a, b) => b - a)
          return (lighter + 0.05) / (darker + 0.05)
        }
        const background = token('--color-bg')
        const accent = token('--color-accent')
        const button = getComputedStyle(document.querySelector('aside button.bg-accent'))
        return {
          background, accent, surface: token('--color-surface'),
          meta: document.querySelector('meta[name="theme-color"]').content,
          scheme: getComputedStyle(root).colorScheme,
          bodyContrast: contrast(token('--color-ink'), background),
          secondaryContrast: contrast(token('--color-ink-soft'), background),
          buttonContrast: contrast(button.color, button.backgroundColor),
          color: rgb(accent),
          buttonColor: rgb(getComputedStyle(document.querySelector('aside button.bg-accent')).backgroundColor),
          navigationColor: rgb(getComputedStyle(document.querySelector('aside nav button')).color),
          secondaryColor: rgb(token('--color-ink-soft')),
          sidebarColor: rgb(getComputedStyle(document.querySelector('aside')).backgroundColor),
          backgroundColor: rgb(background),
          headerHeight: document.querySelector('.story-topbar').getBoundingClientRect().height,
        }
      })
      expect(appearance.meta).toBe(appearance.background)
      expect(appearance.scheme).toBe(mode)
      expect(appearance.bodyContrast).toBeGreaterThanOrEqual(4.5)
      expect(appearance.secondaryContrast).toBeGreaterThanOrEqual(4.5)
      expect(appearance.buttonContrast).toBeGreaterThanOrEqual(4.5)
      expect(appearance.buttonColor).toEqual(appearance.color)
      expect(appearance.navigationColor).toEqual(appearance.secondaryColor)
      expect(appearance.sidebarColor).toEqual(appearance.backgroundColor)
      expect(appearance.headerHeight).toBe(52)
      if (id === 'teal') expect(appearance.color[1]).toBeGreaterThan(appearance.color[0])
      if (id === 'blue') expect(appearance.color[2]).toBeGreaterThan(appearance.color[1])
      if (id === 'rose') expect(appearance.color[0]).toBeGreaterThan(appearance.color[2])
      if (id === 'classic') {
        expect(appearance.background).toBe(mode === 'dark' ? '#1b1712' : '#f6f3ec')
        expect(appearance.surface).toBe(mode === 'dark' ? '#241f18' : '#fffdf7')
        expect(appearance.accent).toBe(mode === 'dark' ? '#d2a55e' : '#9a6425')
      }
      await page.screenshot({ path: test.info().outputPath(`${id}-${mode}.png`) })
      await page.getByRole('button', { name: '查看变量', exact: true }).click()
      const panelHeader = page.getByRole('dialog').locator('header')
      await expect(panelHeader).toBeVisible()
      expect(await panelHeader.evaluate(el =>
        getComputedStyle(el).getPropertyValue('--color-accent').trim(),
      )).toBe(appearance.accent)
    })
  }
}
