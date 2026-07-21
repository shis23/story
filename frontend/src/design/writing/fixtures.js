/**
 * 写作屏 fixture（纯假数据，无任何 store 依赖）。
 * message shape 对齐真实数据结构（见 stores/writing.js 与后端会话节点）：
 *   { id, role, role_label, active_variant, variants: [{ content, display_content, status, provenance }] }
 * pipeline shape 对齐 writingStore.pipeline：
 *   { state, director: {status,detail,output}, subagents: [{id,name,emoji,status,progress,output}],
 *     editor: {status,detail,output}, quality: {passed,warningCount,warnings} }
 */

const para1 = '雨是在黄昏时分落下来的，先是试探性的几滴，敲在巷口的铁皮棚顶上，像是有人在远处用指节轻轻叩门。林述把风衣领子竖起来，拐进了那条他走了七年的窄巷。'
const para2 = '巷子比记忆里更暗。墙根的苔藓在雨水里泛出一种近乎发光的绿，他忽然意识到，自己已经有三年没有在这条巷子里见过第二个人了。'
const para3 = '然后他听见了脚步声。不是回声——回声不会在人停下的时候也停下。'

const variantA = {
  content: `${para1}\n\n${para2}\n\n${para3}`,
  display_content: `${para1}\n\n${para2}\n\n${para3}`,
  status: 'final',
  provenance: { seed: 42137, agents: ['director', 'subagent:lin', 'editor'] },
}
const variantB = {
  content: `${para2}\n\n${para3}\n\n林述没有回头。七年刑警的本能告诉他，回头是最昂贵的动作——它用背影交换信息，用脖颈交换时间。`,
  display_content: `${para2}\n\n${para3}\n\n林述没有回头。七年刑警的本能告诉他，回头是最昂贵的动作——它用背影交换信息，用脖颈交换时间。`,
  status: 'candidate',
  provenance: { seed: 88201, agents: ['director', 'editor'] },
}
const variantC = {
  content: `${para1}\n\n${para3}\n\n"林警官。"那声音说，"别来无恙。"`,
  display_content: `${para1}\n\n${para3}\n\n"林警官。"那声音说，"别来无恙。"`,
  status: 'discarded',
  provenance: { seed: 10933, agents: ['director', 'editor'] },
}

export const fxMessagesWritten = [
  {
    id: 'n1',
    role: 'user',
    role_label: '我',
    active_variant: 0,
    variants: [
      {
        content: '写一段雨夜巷子里的悬念开场，主角是退休刑警林述。',
        display_content: '写一段雨夜巷子里的悬念开场，主角是退休刑警林述。',
        status: 'final',
        provenance: null,
      },
    ],
  },
  {
    id: 'n2',
    role: 'assistant',
    role_label: '林述 · 旁白',
    active_variant: 0,
    variants: [variantA, variantB, variantC],
  },
]

export const fxPipelineDone = {
  state: 'done',
  director: { status: 'done', detail: '节拍 3 · 悬念递进', output: '【导演规划】\n1. 建立雨夜巷子的空间与感官（视觉→听觉）\n2. 用"三年无人"制造异常感\n3. 脚步声收束，停在不回头的瞬间' },
  subagents: [
    { id: 'lin', name: '林述（角色）', emoji: '🕵', status: 'done', progress: 100, output: '角色声口采样：短句、职业本能、克制的恐惧。' },
    { id: 'rain', name: '场景（雨巷）', emoji: '🌧', status: 'done', progress: 100, output: '场景要素：铁皮棚顶、苔藓、窄巷声学。' },
  ],
  editor: { status: 'done', detail: '成文 3 段 · 212 字', output: para1 + '\n\n' + para2 },
  quality: { passed: true, warningCount: 0, warnings: [] },
}

export const fxPipelineStreaming = {
  state: 'running',
  director: { status: 'done', detail: '节拍 3 · 悬念递进', output: '【导演规划】\n1. 建立雨夜巷子的空间与感官\n2. 用"三年无人"制造异常感\n3. 脚步声收束' },
  subagents: [
    { id: 'lin', name: '林述（角色）', emoji: '🕵', status: 'done', progress: 100, output: '' },
    { id: 'rain', name: '场景（雨巷）', emoji: '🌧', status: 'running', progress: 62, output: '' },
  ],
  editor: { status: 'running', detail: '', output: `${para1}\n\n雨是在黄昏时分落下来的` },
  quality: null,
}

export const fxGreetings = [
  { label: '开场一 · 雨巷', content: `${para1}\n\n${para3}` },
  { label: '开场二 · 档案室', content: '档案室的灯管发出细微的嗡鸣。林述把那份编号 0417 的卷宗推到桌子中央，灰尘在光柱里浮起来，像一场迟到了二十年的雪。' },
  { label: '开场三 · 墓园', content: '他每年只来两次。清明，和她的生日。墓碑上的照片已经有些褪色，但那双眼睛看人时微微偏头的样子，和巷子里那个声音一模一样。' },
]

export const fxUserMessageOnly = [fxMessagesWritten[0]]

/** 页题/元信息（接线：Campaign 名 / 会话派生） */
export const fxPageTitle = '第一章 · 雨巷'
export const fxDuration = '用时 8 分钟'
