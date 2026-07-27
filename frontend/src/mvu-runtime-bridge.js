export function getTrustedMvuRuntimeMessage(event, runtimeWindow) {
  if (!event || !runtimeWindow || event.source !== runtimeWindow) {
    return null
  }

  const data = event.data
  if (!data || typeof data.type !== 'string' || !data.type.startsWith('mvu:')) {
    return null
  }

  return data
}

export function hasTauriRuntimeBridge(hostWindow) {
  return Boolean(hostWindow && hostWindow.__TAURI_INTERNALS__)
}
