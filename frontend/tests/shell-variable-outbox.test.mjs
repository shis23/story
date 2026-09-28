import test from 'node:test'
import assert from 'node:assert/strict'
import {
  isReservedNamespace,
  toJsonValue,
  persistShellVariableWrite,
  createVariableWriteAudit,
} from '../src/utils/shellVariableOutbox.js'

test('toJsonValue serializes plain data', () => {
  assert.equal(toJsonValue('a'), 'a')
  assert.equal(toJsonValue(1), 1)
  assert.deepEqual(toJsonValue({ x: 1 }), { x: 1 })
  assert.equal(toJsonValue(undefined), null)
})

test('persistShellVariableWrite writes campaign by default', async () => {
  const calls = []
  const res = await persistShellVariableWrite({
    campaignId: 'c1',
    instanceId: null,
    key: 'hp',
    value: 12,
    setCampaignVariable: async (cid, key, value) => {
      calls.push(['camp', cid, key, value])
    },
    setCharacterVariable: async () => {
      throw new Error('should not instance')
    },
  })
  assert.equal(res.ok, true)
  assert.equal(res.scope, 'campaign')
  assert.deepEqual(calls, [['camp', 'c1', 'hp', 12]])
})

test('persistShellVariableWrite honors instance: prefix', async () => {
  const calls = []
  const res = await persistShellVariableWrite({
    campaignId: 'c1',
    instanceId: 'fallback',
    key: 'instance:inst-9:mood',
    value: 'calm',
    setCampaignVariable: async () => {
      throw new Error('should not camp')
    },
    setCharacterVariable: async (cid, iid, key, value) => {
      calls.push([cid, iid, key, value])
    },
  })
  assert.equal(res.ok, true)
  assert.equal(res.scope, 'instance')
  assert.deepEqual(calls, [['c1', 'inst-9', 'mood', 'calm']])
})

test('persistShellVariableWrite fails without campaign', async () => {
  const res = await persistShellVariableWrite({
    campaignId: null,
    key: 'x',
    value: 1,
    setCampaignVariable: async () => {},
    setCharacterVariable: async () => {},
  })
  assert.equal(res.ok, false)
  assert.match(res.error, /no active campaign/)
})

test('createVariableWriteAudit rings', () => {
  const audit = createVariableWriteAudit(2)
  audit.push({ key: 'a' })
  audit.push({ key: 'b' })
  audit.push({ key: 'c' })
  assert.deepEqual(
    audit.list().map((x) => x.key),
    ['c', 'b'],
  )
})

// M-6 命名空间守卫（M-04 的 MvuJsRuntime 镜像引用同一判定）
test('isReservedNamespace only matches the __storyforge prefix', () => {
  assert.equal(isReservedNamespace('__storyforge_card_shell_variables'), true)
  assert.equal(isReservedNamespace('__storyforge'), true)
  assert.equal(isReservedNamespace('_storyforge_card'), false)
  assert.equal(isReservedNamespace('hp'), false)
  assert.equal(isReservedNamespace(null), false)
  assert.equal(isReservedNamespace(undefined), false)
})

// M-28a：前缀拆分后的空 writeKey 必须 fail closed，不得落到 set*Variable
test('persistShellVariableWrite rejects an empty key after the instance: prefix split', async () => {
  const calls = []
  const logs = []
  const res = await persistShellVariableWrite({
    campaignId: 'c1',
    instanceId: null,
    key: 'instance:inst-9:',
    value: 1,
    setCampaignVariable: async () => calls.push('campaign'),
    setCharacterVariable: async (...args) => calls.push(['instance', ...args]),
    log: async (level, message) => logs.push([level, message]),
  })

  assert.equal(res.ok, false)
  assert.equal(res.scope, 'instance')
  assert.equal(res.key, '')
  assert.equal(res.error, 'empty key')
  assert.deepEqual(calls, [])
  assert.match(logs[0][1], /empty key after prefix split/)
})

test('persistShellVariableWrite rejects empty keys on every prefix shape', async () => {
  const calls = []
  const write = (key, instanceId) => persistShellVariableWrite({
    campaignId: 'c1',
    instanceId,
    key,
    value: 1,
    setCampaignVariable: async () => calls.push('campaign'),
    setCharacterVariable: async () => calls.push('instance'),
  })

  for (const [key, instanceId] of [
    ['inst:', 'iid-1'],
    ['campaign:', null],
    ['campaign:   ', null],
    ['instance:iid-1:   ', null],
  ]) {
    const res = await write(key, instanceId)
    assert.equal(res.ok, false, `${key} must be rejected`)
    assert.equal(res.error, 'empty key')
  }
  assert.deepEqual(calls, [])

  // 非空后缀仍然写入（不误伤正常路径）
  const ok = await write('campaign:hp', null)
  assert.equal(ok.ok, true)
  assert.equal(ok.key, 'hp')
  assert.deepEqual(calls, ['campaign'])
})
