// Mock 数据 —— 让原型可交互，不接真后端

// 当前角色卡（导入自 ST 的样子）
export const currentCharacter = {
  name: '艾莉娅',
  avatar: '🌿',
  tagline: '雨林游侠，沉默寡言的守护者',
  description: '出身南方雨林的精灵游侠，擅长弓与潜行。因故乡被焚而流浪，对火与机械怀有戒心。',
}

// 对话历史（演示对话树 + 分支）
export const messages = [
  {
    id: 'm1',
    role: 'assistant',
    role_label: '艾莉娅',
    active_variant: 0,
    variants: [
      {
        id: 'v1',
        content: '雨下了一整夜。我蹲在屋檐下，看着水珠顺着苔藓滴落，听见远处森林深处传来若有若无的低吼。那不是野兽的声音——是机械。',
        status: 'final',
        provenance: { seed: 12345, profile: '小说预设v2', agents: ['导演', '艾莉娅', '编剧'] },
      },
    ],
  },
  {
    id: 'm2',
    role: 'user',
    role_label: '我',
    active_variant: 0,
    variants: [
      {
        id: 'v2',
        content: '我握紧了剑柄，低声问："有几个？"',
        status: 'final',
        provenance: null,
      },
    ],
  },
  {
    id: 'm3',
    role: 'assistant',
    role_label: '艾莉娅',
    active_variant: 0, // 当前选中的版本
    variants: [
      {
        id: 'v3a',
        content: '艾莉娅竖起三根手指，又比了个"包抄"的手势。她的眼神没有看我，始终盯着雨幕之外那片漆黑的林缘。雨水顺着她的发梢滑落，她连眨眼都没有。"三个，"她终于开口，声音轻得几乎被雨盖过，"两个从东，一个绕到了河那边。他们想把我们逼进死路。"',
        status: 'final',
        provenance: { seed: 88210, profile: '小说预设v2', agents: ['导演', '艾莉娅', '编剧'] },
      },
      {
        id: 'v3b',
        content: '"三个。"艾莉娅的声音冷而短促。她没有回头，弓已在手，箭搭上了弦。雨打在她脸上，她像感觉不到一样。"两个东边，一个摸到了河后面。他们打算合围。"',
        status: 'discarded', // 这是被重 roll 掉的旧版本
        provenance: { seed: 77210, profile: '小说预设v2', agents: ['导演', '艾莉娅', '编剧'] },
      },
    ],
  },
]

// 写作意图输入（演示用）
export const sampleIntent = '写一场雨夜林中遭遇机械追兵的紧张戏，要有潜行与对峙的张力'

// 提示词模块库（演示按钮切换用）
export const promptModules = {
  perspective: [
    { id: 'p1', name: '第一人称', selected: true },
    { id: 'p2', name: '第二人称', selected: false },
    { id: 'p3', name: '第三人称', selected: false },
    { id: 'p4', name: '群像', selected: false },
  ],
  style: [
    { id: 's1', name: '白描', selected: true },
    { id: 's2', name: '轻小说', selected: false },
    { id: 's3', name: '网文', selected: false },
    { id: 's4', name: '古风', selected: false },
    { id: 's5', name: '魔幻现实', selected: false },
  ],
  cot: [
    { id: 'c1', name: 'Gemini', selected: false },
    { id: 'c2', name: 'Claude', selected: true },
    { id: 'c3', name: 'GLM', selected: false },
    { id: 'c4', name: 'DeepSeek', selected: false },
  ],
  quality: [
    { id: 'q1', name: '杀八股', selected: true },
    { id: 'q2', name: '抗抢话', selected: true },
    { id: 'q3', name: '抗绝望', selected: false },
    { id: 'q4', name: '防重复', selected: true },
    { id: 'q5', name: '反神化', selected: false },
  ],
}

// LLM 连接列表
export const connections = [
  { id: 'cn1', name: 'DeepSeek 官方', model: 'deepseek-chat' },
  { id: 'cn2', name: '我的 Gemini', model: 'gemini-2.5-pro' },
  { id: 'cn3', name: 'Claude', model: 'claude-sonnet-4-5' },
]
