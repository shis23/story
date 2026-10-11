/**
 * dsh-plugin-storyforge/agent — StoryForge 预设里的 agent 作用域插件。
 *
 * 只出现在 `storyforge` 预设的 plugins 列表里，因此只在 StoryForge 会话的 agent
 * 作用域内激活；`agent/request` 与 `agent/created` 都按 agent 作用域分发，这里的监听
 * 天然只作用于 StoryForge 会话（分叉出的子会话继承预设，同样覆盖）。
 *
 * - 每次模型请求前把调用配置钉回 storyforge/pipeline：用户在会话里改了模型、或者
 *   分叉出的子会话被重置成全局默认模型，下一次请求都会被拉回 StoryForge 管线。
 *   代价：改过的选择永远等不到"同模型请求头"来消费，DSH 会每轮追加一条换模型通知
 *   （`source.kind: 'model-selection'`）。它的 pre-step 监听用 prepend 注册在我们之后，
 *   插件剥不掉（P1 实测），所以客户端在 StoryForge 会话里盖住模型选择器，适配器取输入时
 *   跳过通知；`/model` 命令仍是残留路径。
 * - 记录 agent 创建事件；带 `parentSession` + `isSeeded` 的是分叉子会话，通知宿主半边
 *   建立映射（P3 再对应到 Campaign 分支）。
 */

import { MODEL, PROVIDER, log, notifyFork, recordLifecycle } from './shared.js'

export const name = 'storyforge-agent'

export function apply(ctx) {
  ctx.on('agent/request', async (_payload, next) => {
    const resolved = await next()
    if (resolved.provider === PROVIDER && resolved.model === MODEL) return resolved
    log(`agent/request: ${resolved.provider}/${resolved.model} -> ${PROVIDER}/${MODEL}`)
    // 推理档位属于原模型，换到 StoryForge 后丢弃。
    const { reasoningEffort: _dropped, ...rest } = resolved
    return { ...rest, provider: PROVIDER, model: MODEL }
  })

  ctx.on('agent/created', ({ agent, source }) => {
    const header = agent.session.header
    const event = {
      kind: 'agent-created',
      sessionId: header.id,
      source,
      parentSession: header.parentSession ?? null,
      isSeeded: header.isSeeded === true,
    }
    recordLifecycle(event)
    log(`agent/created: ${event.sessionId} source=${source} parent=${event.parentSession} seeded=${event.isSeeded}`)
    if (event.parentSession && event.isSeeded) notifyFork(event.sessionId, event.parentSession)
  })
}
