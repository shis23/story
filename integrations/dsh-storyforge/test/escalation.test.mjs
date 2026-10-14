// 自动路由升档确认：问题文字、回答 → 显式模式、取消与没法问时的说明。
import assert from 'node:assert/strict'
import test from 'node:test'

import { QUESTION_ID, confirmEscalation, escalationQuestion, modeFromAnswer } from '../lib/escalation.js'

const DATA = {
  mode: 'sequential_crew',
  reason: 'actor_count',
  estimate: '2+N 次顺序编排 + 2 次廉价记账',
  continuation_estimate: '1 次正文 + 2 次廉价记账',
}
const answer = (selected, custom) => ({ answers: [{ id: QUESTION_ID, selected, ...(custom === undefined ? {} : { custom }) }] })
const harnessError = (code) => Object.assign(new Error(code), { code })

test('the question names the suggested mode, the reason and both estimates', () => {
  const { question, labels } = escalationQuestion(DATA)
  assert.equal(question.id, QUESTION_ID)
  assert.match(question.question, /群像（顺序子 Agent）/)
  assert.match(question.detail, /4 个或更多角色/)
  assert.match(question.detail, /管线设置/)
  assert.deepEqual(
    question.options.map((option) => option.label),
    [labels.escalate, labels.continuation, labels.cancel],
  )
  assert.match(question.options[0].description, /2\+N 次顺序编排/)
  assert.match(question.options[1].description, /1 次正文/)
  // 没有 multiSelect / intent：DSH 按普通单选渲染。
  assert.equal(question.multiSelect, undefined)
  assert.equal(question.intent, undefined)
})

test('answers map to an explicit mode; cancel, free text and stray answers do not', () => {
  const { labels } = escalationQuestion(DATA)
  assert.equal(modeFromAnswer(answer([labels.escalate]), DATA, labels), 'sequential_crew')
  assert.equal(modeFromAnswer(answer([labels.continuation]), DATA, labels), 'continuation')
  assert.equal(modeFromAnswer(answer([labels.cancel]), DATA, labels), null)
  assert.equal(modeFromAnswer(answer([], '换个写法'), DATA, labels), null)
  assert.equal(modeFromAnswer(answer([labels.escalate], '补充'), DATA, labels), null)
  assert.equal(modeFromAnswer(answer([labels.escalate, labels.continuation]), DATA, labels), null)
  assert.equal(modeFromAnswer({ answers: [{ id: 'other', selected: [labels.escalate] }] }, DATA, labels), null)
  assert.equal(modeFromAnswer(undefined, DATA, labels), null)
  // sidecar 给了认不出的模式：不把它原样发回去。
  const odd = { ...DATA, mode: 'huge' }
  const oddLabels = escalationQuestion(odd).labels
  assert.equal(modeFromAnswer(answer([oddLabels.escalate]), odd, oddLabels), null)
})

test('confirmEscalation returns the chosen mode and explains cancel or a missing UI', async () => {
  const { labels } = escalationQuestion(DATA)
  const asked = []
  const ask = (selected) => async (question) => {
    asked.push(question)
    return answer(selected)
  }
  assert.equal(await confirmEscalation(DATA, ask([labels.escalate])), 'sequential_crew')
  assert.equal(await confirmEscalation(DATA, ask([labels.continuation])), 'continuation')
  assert.equal(asked.length, 2)
  await assert.rejects(confirmEscalation(DATA, ask([labels.cancel])), /已取消这一轮/)
  // 用户关掉问题直接发言 = 放弃这次确认。
  await assert.rejects(confirmEscalation(DATA, async () => { throw harnessError('ASK_CANCELLED') }), /已取消这一轮/)
  // 没有可问的界面：照旧报错，指到首页设置。
  await assert.rejects(confirmEscalation(DATA, () => null), /管线设置/)
  await assert.rejects(confirmEscalation(DATA, async () => { throw harnessError('NO_PROVIDER') }), /管线设置/)
  // 中止交给调用方按停止处理：原样抛出。
  await assert.rejects(confirmEscalation(DATA, async () => { throw harnessError('ASK_ABORTED') }), (error) => error.code === 'ASK_ABORTED')
})
