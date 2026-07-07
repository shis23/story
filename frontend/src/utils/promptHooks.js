import { canModifyPrompt } from '../plugin-bridge.js'

export async function emitPromptHookEventAndWaitForPlugins(plugins, hostRefs, event, data = {}, options = {}) {
  let payload = data

  for (const plugin of plugins || []) {
    if (!canModifyPrompt(plugin)) continue

    const host = hostRefs?.get?.(plugin.id)
    if (host?.emitPluginEventAndWait) {
      try {
        const nextPayload = await host.emitPluginEventAndWait(event, payload)
        if (nextPayload !== undefined) {
          payload = nextPayload
        }
      } catch (error) {
        try {
          options?.onError?.(error, plugin)
        } catch {
          // Error reporting should not make prompt hooks fail closed.
        }
      }
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
