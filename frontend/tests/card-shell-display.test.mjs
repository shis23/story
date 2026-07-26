import test from 'node:test'
import assert from 'node:assert/strict'
import {
  extractShellMountsFromDisplay,
  classifyShellUrl,
  resolveShellSurfaces,
  segmentShellContent,
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

// ─── segmentShellContent（原地渲染分段器）───────────────────────────────

test('segmentShellContent keeps mid-text shells at their original position', () => {
  const doc = '<body class="battle"><div id="app"></div><script>boot()</script></body>'
  const display = `叙事A。\n$('body').load('${STATUS}')\n叙事B。\n${doc}\n叙事C。`

  const segs = segmentShellContent(display)
  assert.deepEqual(
    segs.map((s) => (s.type === 'text' ? 'text' : `${s.mode}:${s.kind}`)),
    ['text', 'load:status', 'text', 'inline:message_html', 'text'],
  )
  assert.equal(segs[0].content, '叙事A。')
  assert.equal(segs[1].url, STATUS)
  assert.equal(segs[2].content, '叙事B。')
  assert.match(segs[3].html, /boot\(\)/)
  assert.equal(segs[4].content, '叙事C。')
})

test('segmentShellContent claims fence glue as one region without bare duplicates', () => {
  const display = [
    '前文。',
    '```',
    '<body>',
    `<script>$('body').load('${HOME}')</script>`,
    '</body>',
    '```',
    '后文。',
  ].join('\n')

  const segs = segmentShellContent(display)
  assert.deepEqual(
    segs.map((s) => (s.type === 'text' ? s.content : s.url)),
    ['前文。', HOME, '后文。'],
  )
})

test('segmentShellContent renders duplicate urls once, at first occurrence', () => {
  const display = `A\n$('body').load('${HOME}')\nB\n$('body').load('${HOME}')\nC`
  const segs = segmentShellContent(display)
  const shells = segs.filter((s) => s.type === 'shell')
  assert.equal(shells.length, 1)
  // 首次出现位置在 A 之后、B 之前
  assert.deepEqual(
    segs.map((s) => (s.type === 'text' ? s.content : 'shell')),
    ['A', 'shell', 'B', 'C'],
  )
})

test('segmentShellContent returns byte-identical single segment for plain messages', () => {
  const plain = '普通叙事。\n\n\n\n多个空行必须原样保留。  尾部空格也是。  '
  const segs = segmentShellContent(plain)
  assert.equal(segs.length, 1)
  assert.equal(segs[0].type, 'text')
  assert.equal(segs[0].content, plain)

  // 无脚本静态 HTML 也走无壳快路径
  const staticHtml = '前文<body><p>静态</p></body>后文'
  const staticSegs = segmentShellContent(staticHtml)
  assert.equal(staticSegs.length, 1)
  assert.equal(staticSegs[0].content, staticHtml)
})

test('segmentShellContent offsets are append-stable, rewrite may shift them once', () => {
  const doc = '<body><script>ui()</script></body>'
  const base = `叙事开头。\n${doc}\n继续`

  const before = segmentShellContent(base)
  const after = segmentShellContent(base + '叙事，流式追加了更多文字。')
  const shellBefore = before.find((s) => s.type === 'shell')
  const shellAfter = after.find((s) => s.type === 'shell')
  // 流式尾部追加：已出现壳段的 start 偏移与内容不变（key 稳定，iframe 不重挂）
  assert.equal(shellAfter.start, shellBefore.start)
  assert.equal(shellAfter.html, shellBefore.html)

  // 流结束的全文改写（前缀变化）：偏移允许移动 → 内联壳按 start 作 key
  // 允许一次性重挂。这是设计内行为，不是 bug。
  const rewritten = segmentShellContent(`【展示正则改写后的新前缀，更长的开场】\n${doc}\n继续`)
  const shellRewritten = rewritten.find((s) => s.type === 'shell')
  assert.notEqual(shellRewritten.start, shellBefore.start)
  assert.equal(shellRewritten.html, shellBefore.html)
})
