/**
 * 自动路由升档确认。
 *
 * 生成模式选「自动路由」时，StoryForge 判断这一轮该升到高成本的群像档，sidecar 就以
 * `confirm_generation_mode` 拒绝本轮（没调模型、没写存档），错误的 `data` 带建议模式与
 * 预计调用量。插件在会话的输入框处问用户：用建议的档、按续写，还是取消；选定后带显式
 * 模式重发。没有可问的界面时照旧报错，提示到首页固定生成模式。
 */

export const CONFIRM_GENERATION_MODE = 'confirm_generation_mode'
export const QUESTION_ID = 'storyforge-generation-mode'

/** 生成模式（auto = StoryForge 自动路由；其余显式选择，不再二次确认）。 */
export const GENERATION_MODE_LABELS = Object.freeze({
  auto: '自动路由',
  continuation: '续写（单次执笔）',
  duet: '双人戏',
  big_scene: '群像（并行子 Agent）',
  sequential_crew: '群像（顺序子 Agent）',
})

/** 路由理由（`GenerationRouteReason`）→ 给用户看的说明。 */
const REASONS = Object.freeze({
  actor_count: '这一轮点到了 4 个或更多角色',
  large_scene_intent: '这一轮有 3 个以上角色，输入像是大场面',
  multiple_imminent_tasks: '这一轮有 3 个以上角色，且有多个临近的任务',
})

const modeLabel = (mode) => GENERATION_MODE_LABELS[mode] ?? mode ?? '群像'

/** 确认问题（DSH `userQuestions` 的单个问题）与三个选项的文字。 */
export function escalationQuestion(data) {
  const mode = modeLabel(data?.mode)
  const labels = {
    escalate: `这轮用${mode}`,
    continuation: '这轮按续写',
    cancel: '取消这一轮',
  }
  const reason = REASONS[data?.reason] ?? '自动路由判断这一轮场面较大'
  return {
    labels,
    question: {
      id: QUESTION_ID,
      header: '生成模式',
      question: `自动路由建议这一轮用${mode}，要继续吗？`,
      detail: `${reason}。本轮还没开始，没有调用模型。想以后不再问，可以在 StoryForge 首页「管线设置」里固定生成模式。`,
      options: [
        { label: labels.escalate, description: `预计 ${data?.estimate ?? '多次模型调用'}` },
        { label: labels.continuation, description: `单次执笔，预计 ${data?.continuation_estimate ?? '1 次正文'}` },
        { label: labels.cancel, description: '不写这一轮，存档不变' },
      ],
    },
  }
}

/** 用户的回答 → 本轮的显式模式；选了取消或填了自由文本时返回 null。 */
export function modeFromAnswer(answer, data, labels) {
  const items = (answer?.answers ?? []).filter((entry) => entry?.id === QUESTION_ID)
  const item = items.length === 1 ? items[0] : undefined
  if (!item || item.custom !== undefined || item.selected?.length !== 1) return null
  const [choice] = item.selected
  if (choice === labels.escalate && typeof data?.mode === 'string' && GENERATION_MODE_LABELS[data.mode]) return data.mode
  if (choice === labels.continuation) return 'continuation'
  return null
}

const cancelled = () => new Error('已取消这一轮：没有调用模型，存档没有变化。')

/**
 * 问用户这一轮怎么写，返回显式生成模式。
 *
 * `ask(question)` 返回 DSH 的回答；没有可问的界面时返回 null（或抛 `NO_PROVIDER`）。
 * 用户取消、改为直接发言、没法问时抛出说明（本轮不写，存档不变）；中止信号触发时
 * DSH 抛 `ASK_ABORTED`，原样抛出交给调用方按中止处理。
 */
export async function confirmEscalation(data, ask) {
  const { question, labels } = escalationQuestion(data)
  let answer
  try {
    answer = await ask(question)
  } catch (error) {
    // 用户关掉问题改为直接发言（DSH plan-mode 同样把它当作放弃本次确认）。
    if (error?.code === 'ASK_CANCELLED') throw cancelled()
    if (error?.code !== 'NO_PROVIDER') throw error
    answer = null
  }
  if (answer == null) {
    throw new Error(
      `自动路由建议这一轮用${modeLabel(data?.mode)}（预计 ${data?.estimate ?? '多次模型调用'}），但这里没法弹出确认，本轮没有开始。` +
        '请到 StoryForge 首页「管线设置」里选定生成模式后重发。',
    )
  }
  const mode = modeFromAnswer(answer, data, labels)
  if (!mode) throw cancelled()
  return mode
}

/** 思考块里的一行：正在等用户确认 / 用户选定了哪一档。 */
export function describeEscalation(data) {
  return `【确认】自动路由建议这一轮用${modeLabel(data?.mode)}（预计 ${data?.estimate ?? '多次模型调用'}），请在输入框处选择。\n`
}

export function describeChosenMode(mode) {
  return `【确认】这一轮用${modeLabel(mode)}。\n`
}
