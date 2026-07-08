import test from 'node:test'
import assert from 'node:assert/strict'
import { makeForkCampaignName } from '../src/utils/forkCampaignName.js'

test('makeForkCampaignName 用传入的 base 生成「分支」后缀名', () => {
  const name = makeForkCampaignName('寒渊谜塔')
  assert.ok(name.startsWith('寒渊谜塔 分支 '), `应以「寒渊谜塔 分支 」开头，实际: ${name}`)
  // 时间戳部分非空
  const stamp = name.slice('寒渊谜塔 分支 '.length)
  assert.ok(stamp.length > 0, '时间戳不应为空')
})

test('makeForkCampaignName base 为空时兜底 Campaign', () => {
  const name = makeForkCampaignName('')
  assert.ok(name.startsWith('Campaign 分支 '), `空 base 应兜底 Campaign，实际: ${name}`)
})

test('makeForkCampaignName base 为 null/undefined 时兜底 Campaign', () => {
  assert.ok(makeForkCampaignName(null).startsWith('Campaign 分支 '))
  assert.ok(makeForkCampaignName(undefined).startsWith('Campaign 分支 '))
})

test('makeForkCampaignName 时间戳格式为 zh-CN 月/日 时:分', () => {
  // 固定时间避免与时区相关断言失败：只校验格式而非精确值
  const name = makeForkCampaignName('X')
  // 形如 "X 分支 07/08 14:30"（zh-CN 默认 24h，含前导零）
  assert.match(name, /^X 分支 \d{2}\/\d{2} \d{2}:\d{2}$/)
})

test('makeForkCampaignName 不同 base 产生不同前缀', () => {
  const a = makeForkCampaignName('第一周目')
  const b = makeForkCampaignName('第二周目')
  assert.ok(a.startsWith('第一周目 分支 '))
  assert.ok(b.startsWith('第二周目 分支 '))
  assert.notEqual(a, b)
})
