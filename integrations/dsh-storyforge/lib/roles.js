/**
 * 底层模型调用的管线角色（首页「底层模型调用」表用）。
 *
 * LLM 层的请求不带角色（桌面版的调用日志同样没有），只能看系统提示词里的身份句。
 * 取**最早出现**的那一句：提示词模块可能在前面拼接内容，角色设定里也可能提到别的
 * 身份，最早的身份句才是这次调用自己的。
 */

const IDENTITIES = [
  ['你是写作导演', '导演'],
  ['你是编剧', '编剧'],
  ['你是执笔者', '执笔'],
  ['你是对手戏场记定稿员', '定稿'],
  ['你是本轮剧情总结助手', '摘要'],
  ['你是后处理助手', '后处理'],
  ['你是卡内角色识别助手', '角色识别'],
  ['你是剧情纪要压缩助手', '纪要压缩'],
  ['你是卡内状态栏分析助手', 'MVU 分析'],
]

const ACTOR = '你是角色 '
const CREW = /你是顺序剧组演员 ([^，,。\s]+)/

/** 消息列表（协议形状 `{ role, content }`）里所有 system 文本。 */
function systemText(messages) {
  return (messages ?? [])
    .filter((message) => message?.role === 'system' && typeof message.content === 'string')
    .map((message) => message.content)
    .join('\n')
}

/**
 * 一次调用的角色标签：`导演` / `角色 · 苏沁` / `编剧` / `执笔` / `摘要` / `后处理` …；
 * 认不出时返回 `其它`。
 */
export function classifyCall(messages) {
  const system = systemText(messages)
  let best = null
  const consider = (index, label) => {
    if (index >= 0 && (best === null || index < best.index)) best = { index, label }
  }
  for (const [phrase, label] of IDENTITIES) consider(system.indexOf(phrase), label)

  const actor = system.indexOf(ACTOR)
  if (actor >= 0) {
    // 名字取最后一处「你是角色 X。」：子 Agent 基础模板开头还有一句没替换的 `{name}`。
    const start = system.lastIndexOf(ACTOR) + ACTOR.length
    const end = system.indexOf('。', start)
    const name = end > start ? system.slice(start, end).trim() : ''
    consider(actor, name && name !== '{name}' ? `角色 · ${name}` : '角色')
  }
  const crew = CREW.exec(system)
  if (crew) consider(crew.index, `角色 · ${crew[1]}`)

  return best?.label ?? '其它'
}
