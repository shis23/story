export function activeVariantForMessageNode(message) {
  if (!message?.variants?.length) return null
  return message.variants[message.active_variant] || message.variants[0] || null
}

export function findLastAssistantConversationNode(messages, conversationId) {
  if (!conversationId || !Array.isArray(messages)) return null

  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const message = messages[i]
    if (!message || message.id === 'editor-streaming' || message.role !== 'assistant') continue

    const activeVariant = activeVariantForMessageNode(message)
    if (!activeVariant?.provenance) continue

    return {
      conversation_id: conversationId,
      node_id: message.id,
    }
  }

  return null
}
