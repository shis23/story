/**
 * dsh-plugin-storyforge/agent — StoryForge 预设里的 agent 作用域插件。
 *
 * 只出现在 `storyforge` 预设的 plugins 列表里，因此只在 StoryForge 会话的 agent
 * 作用域内激活；`agent/request` 是按 agent 作用域分发的 waterfall，这里的监听天然
 * 只作用于 StoryForge 会话。
 *
 * 作用：每次模型请求前把调用配置钉回 storyforge/pipeline。用户在会话里改了模型、
 * 或者分叉出的子会话被重置成全局默认模型，下一次请求都会被拉回 StoryForge 管线。
 */

import { MODEL, PROVIDER, log } from './shared.js'

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
}
