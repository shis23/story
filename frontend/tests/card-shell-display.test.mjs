import test from 'node:test'
import assert from 'node:assert/strict'
import {
  extractShellMountsFromDisplay,
  classifyShellUrl,
  resolveShellSurfaces,
} from '../src/utils/cardShellDisplay.js'

const HOME =
  'https://testingcf.jsdelivr.net/gh/The-poem-of-destiny/FrontEnd-for-destined-journey@1.6.2/dist/home/index.html'
const STATUS =
  'https://testingcf.jsdelivr.net/gh/The-poem-of-destiny/FrontEnd-for-destined-journey@1.6.2/dist/status/index.html'
const CUSTOM =
  'https://testingcf.jsdelivr.net/gh/The-poem-of-destiny/FrontEnd-for-destined-journey@1.6.2/dist/custom_start/index.html'

test('classifyShellUrl maps test-card paths', () => {
  assert.equal(classifyShellUrl(HOME), 'opening_home')
  assert.equal(classifyShellUrl(STATUS), 'status')
  assert.equal(classifyShellUrl(CUSTOM), 'opening_custom')
  // H5：/intro/ 与后端开场关键词对齐（卿卿类开场介绍页）
  assert.equal(
    classifyShellUrl('https://aireckchen-dot.example.com/qingqing/intro/index.html'),
    'opening_custom',
  )
})

test('extractShellMountsFromDisplay finds body.load urls and strips glue', () => {
  const display = [
    '前序叙事。',
    '```',
    '<body>',
    `<script>$('body').load('${STATUS}')</script>`,
    '</body>',
    '```',
    '后序。',
    `$('body').load("${HOME}")`,
  ].join('\n')

  const { mounts, residualText } = extractShellMountsFromDisplay(display)
  assert.equal(mounts.length, 2)
  assert.deepEqual(
    mounts.map((m) => m.kind).sort(),
    ['opening_home', 'status'],
  )
  assert.match(residualText, /前序叙事/)
  assert.match(residualText, /后序/)
  assert.doesNotMatch(residualText, /\$\('body'\)\.load/)
})

test('resolveShellSurfaces fills missing campaign fallbacks', () => {
  const resolved = resolveShellSurfaces([], {
    statusUrl: STATUS,
    openingUrl: HOME,
  })
  assert.equal(resolved.length, 2)
  assert.equal(resolved[0].kind, 'status')
  assert.equal(resolved[1].kind, 'opening_home')
})

test('partitionShellMountsByTrust auto-mounts only card-manifest urls (H3)', async () => {
  const { partitionShellMountsByTrust } = await import('../src/utils/cardShellDisplay.js')
  const attacker = 'https://files.catbox.moe/attacker.html'
  const mounts = [
    { url: HOME, kind: 'opening_home' },
    { url: attacker, kind: 'message_html' },
  ]

  const { allowed, needsConfirmation } = partitionShellMountsByTrust(mounts, [HOME])
  assert.deepEqual(allowed.map((m) => m.url), [HOME])
  assert.deepEqual(needsConfirmation.map((m) => m.url), [attacker])

  // 用户显式放行后可挂载
  const approved = partitionShellMountsByTrust(mounts, [HOME], [attacker])
  assert.equal(approved.needsConfirmation.length, 0)
  assert.equal(approved.allowed.length, 2)

  // 无任何信任上下文（legacy/无 manifest）：全部需要确认，fail closed
  const none = partitionShellMountsByTrust(mounts, null, null)
  assert.equal(none.allowed.length, 0)
  assert.equal(none.needsConfirmation.length, 2)
})

test('extractInlineShellDocsFromDisplay lifts executable docs, keeps static html (H4)', async () => {
  const { extractInlineShellDocsFromDisplay } = await import('../src/utils/cardShellDisplay.js')
  const doc = '<body class="cultivation"><div id="app"></div><script>boot()</script></body>'
  const staticHtml = '<body><p>纯静态段落</p></body>'
  const display = `叙事前文。\n${doc}\n中间叙事。\n${staticHtml}\n结尾。`

  const { docs, residualText } = extractInlineShellDocsFromDisplay(display)
  assert.equal(docs.length, 1)
  assert.equal(docs[0].kind, 'message_html')
  assert.ok(docs[0].inline)
  assert.match(docs[0].html, /boot\(\)/)
  // 可执行文档从 residual 移除；无脚本的静态 HTML 留给 RichContent
  assert.doesNotMatch(residualText, /boot\(\)/)
  assert.match(residualText, /纯静态段落/)
  assert.match(residualText, /叙事前文/)
  assert.match(residualText, /结尾/)

  // 无内联文档时原样返回
  const none = extractInlineShellDocsFromDisplay('普通叙事，无 HTML。')
  assert.equal(none.docs.length, 0)
  assert.equal(none.residualText, '普通叙事，无 HTML。')
})

test('parseStFindRegex + matchesAnyInlineShellTrigger anchor docs to card regex (H4)', async () => {
  const { parseStFindRegex, matchesAnyInlineShellTrigger } =
    await import('../src/utils/cardShellDisplay.js')

  const wrapped = parseStFindRegex('/【修炼界面】([\\s\\S]*?)【\\/修炼界面】/g')
  assert.ok(wrapped instanceof RegExp)
  assert.ok(wrapped.test('【修炼界面】境界：筑基【/修炼界面】'))

  const bare = parseStFindRegex('【战斗系统】')
  assert.ok(bare.test('回合开始【战斗系统】敌方先手'))

  assert.equal(parseStFindRegex(''), null)
  assert.equal(parseStFindRegex('([unclosed'), null)

  const triggers = [
    { label: '修炼界面', trigger: '/【修炼界面】([\\s\\S]*?)【\\/修炼界面】/g' },
    { label: '战斗系统', trigger: '【战斗系统】' },
  ]
  assert.ok(matchesAnyInlineShellTrigger('【修炼界面】…【/修炼界面】', triggers))
  assert.ok(!matchesAnyInlineShellTrigger('普通叙事', triggers))
  assert.ok(!matchesAnyInlineShellTrigger('', triggers))
  assert.ok(!matchesAnyInlineShellTrigger('【修炼界面】…【/修炼界面】', []))
})
