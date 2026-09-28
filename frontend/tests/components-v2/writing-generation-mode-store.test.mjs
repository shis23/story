// W-31 / N-R2-09 回归测试：历史遗留的 `big_scene` 不得成为前端当前档位。
//
// 缺陷：`stores/writing.js` 的 `validGenerationModes` 曾把 `'big_scene'` 当合法值，
// 而产品面目录（`utils/generationModes.js::generationModeCatalog`）只有三档、
// `ComposerBar` 也只渲染这三档 ⇒ 旧 localStorage 里的 `big_scene` 被接受为当前档位，
// 参与 `start_writing` / `allowPartialReroll` 判定，用户处在"看不到选中项"的昂贵模式。
//
// 修复口径：校验集**从目录派生**（单一事实源），`big_scene` 在前端不再是合法档位；
// 解析到它时回退默认档位 `continuation`，且**不重写/不删除** localStorage 里的旧数据。
// 后端仍保留 `BigScene` 兼容模式（`crates/domain/src/generation.rs`、README.md:19、
// docs/AGENT_INTERFACES.md:12），本测试只约束前端产品面。
//
// 为什么放在 tests/components-v2/：`vitest.config.mjs` 的 include 只覆盖
// `tests/components-v2/**/*.test.mjs`（纯 JS 工具测试走 `npm test` 的 node --test）。
// 本文件是 vitest 用例，由 `npm run test:ui` 执行。
import { test, expect, beforeEach, afterEach } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { useWritingStore } from '../../src/stores/writing.js'
import { useCampaignStore } from '../../src/stores/campaign.js'
import { generationModeCatalog } from '../../src/utils/generationModes.js'
import { rerollPolicy } from '../../src/utils/rerollPolicy.js'

const STORAGE_KEY = 'storyforge:generation-mode-by-campaign'

function seedStorage(map) {
  globalThis.localStorage.setItem(STORAGE_KEY, JSON.stringify(map))
}

function readStorage() {
  return globalThis.localStorage.getItem(STORAGE_KEY)
}

/** 新建 pinia + store；store 首次实例化时读 localStorage，故必须在 seed 之后调用。 */
function boot() {
  setActivePinia(createPinia())
  const writing = useWritingStore()
  const campaign = useCampaignStore()
  return { writing, campaign }
}

beforeEach(() => {
  globalThis.localStorage.clear()
})

afterEach(() => {
  globalThis.localStorage.clear()
})

test('目录只有三档，且不含 big_scene', () => {
  expect(generationModeCatalog.map((mode) => mode.value)).toEqual([
    'continuation',
    'duet',
    'sequential_crew',
  ])
})

test('旧 localStorage 里的 big_scene 不再被接受为当前档位（回退默认档位）', () => {
  seedStorage({ 'campaign-a': 'big_scene', 'campaign-b': 'duet' })
  const rawBefore = readStorage()

  const { writing, campaign } = boot()
  campaign.activeCampaign = { id: 'campaign-a', name: 'A' }

  // 修复前这里会是 'big_scene'。这就是本用例的失败控制点。
  expect(writing.generationMode).toBe('continuation')
  // 同一次解析里其它 Campaign 的合法档位不受影响。
  campaign.activeCampaign = { id: 'campaign-b', name: 'B' }
  expect(writing.generationMode).toBe('duet')

  // 非破坏性：读取阶段不重写、不删除用户已存的偏好 blob。
  expect(readStorage()).toBe(rawBefore)
})

test('setGenerationMode 拒绝 big_scene 与未知值，合法三档正常', () => {
  const { writing, campaign } = boot()
  campaign.activeCampaign = { id: 'campaign-a', name: 'A' }

  writing.setGenerationMode('big_scene')
  expect(writing.generationMode).toBe('continuation')
  // 拒绝必须发生在持久化之前：本用例从空存储启动，因此不得产生任何写入。
  expect(readStorage()).toBeNull()

  writing.setGenerationMode('made_up_mode')
  expect(writing.generationMode).toBe('continuation')

  for (const mode of generationModeCatalog.map((entry) => entry.value)) {
    writing.setGenerationMode(mode)
    expect(writing.generationMode).toBe(mode)
    expect(JSON.parse(readStorage())['campaign-a']).toBe(mode)
  }
})

test('用户重新选择档位后，偏好 blob 里不再残留 big_scene', () => {
  seedStorage({ 'campaign-a': 'big_scene' })
  const { writing, campaign } = boot()
  campaign.activeCampaign = { id: 'campaign-a', name: 'A' }

  expect(writing.generationMode).toBe('continuation')
  writing.setGenerationMode('duet')

  expect(JSON.parse(readStorage())).toEqual({ 'campaign-a': 'duet' })
})

test('合法档位跨 store 重建仍然按 Campaign 记忆', () => {
  seedStorage({ 'campaign-a': 'sequential_crew', 'campaign-b': 'big_scene' })

  let { writing, campaign } = boot()
  campaign.activeCampaign = { id: 'campaign-a', name: 'A' }
  expect(writing.generationMode).toBe('sequential_crew')

  // 模拟重启：store 重建后仍从同一 blob 读取。
  ;({ writing, campaign } = boot())
  campaign.activeCampaign = { id: 'campaign-a', name: 'A' }
  expect(writing.generationMode).toBe('sequential_crew')
  campaign.activeCampaign = { id: 'campaign-b', name: 'B' }
  expect(writing.generationMode).toBe('continuation')
})

test('陈旧 big_scene 不会让局部 reroll 判定落到 legacy 昂贵路径', () => {
  seedStorage({ 'campaign-a': 'big_scene' })
  const { writing, campaign } = boot()
  campaign.activeCampaign = { id: 'campaign-a', name: 'A' }

  // 与 `adapter/useWritingScreenAdapter.js` 的 allowPartialReroll 同式判定：
  // 非 campaign 会话 || 当前档位 === 'big_scene'。
  const allowPartialReroll =
    writing.writingMode !== 'campaign' || writing.generationMode === 'big_scene'

  expect(allowPartialReroll).toBe(false)
  expect(rerollPolicy('continuation', 'big_scene', allowPartialReroll)).toEqual({
    editorOnly: false,
    sequentialSuffix: false,
  })
  // 对照：若陈旧值仍被接受（修复前），同一 recorded 模式会走 legacy 局部重 roll。
  expect(rerollPolicy('continuation', 'big_scene', true).editorOnly).toBe(true)
})
