import { canModifyPrompt } from '../plugin-bridge.js'

export async function emitPromptHookEventAndWaitForPlugins(plugins, hostRefs, event, data = {}) {
  let payload = data

  for (const plugin of plugins || []) {
    if (!canModifyPrompt(plugin)) continue

    const host = hostRefs?.get?.(plugin.id)
    if (host?.emitPluginEventAndWait) {
      payload = await host.emitPluginEventAndWait(event, payload)
    }
  }

  return payload
}

export function resolveHookedIntent(payload, fallbackIntent) {
  if (typeof payload?.intent === 'string') {
    return payload.intent
  }
  if (typeof payload?.prompt === 'string') {
    return payload.prompt
  }
  return fallbackIntent
}

export function resolveHookedMessages(payload, fallbackMessages) {
  return Array.isArray(payload?.messages) ? payload.messages : fallbackMessages
}
