/**
 * Prompt hook 审计记录导出。
 *
 * 每个审计记录已由 `buildAuditRecord` 脱敏（只存 type/length/hash，不存原始文本）。
 * 导出函数只做 JSON 序列化 + 元数据包装，不额外收集敏感信息。
 */

/**
 * 将 prompt hook 审计记录导出为可保存的 JSON 字符串。
 *
 * @param {Array<object>} records - 审计记录数组，每条由 `buildAuditRecord` 生成
 * @returns {string} 格式化的 JSON 字符串
 */
export function exportPromptHookAudit(records) {
  const safeRecords = Array.isArray(records) ? records : []
  return JSON.stringify(
    {
      exportedAt: new Date().toISOString(),
      kind: 'prompt_hook_audit_export',
      schema: {
        identity: ['pluginId', 'pluginName', 'event', 'stage'],
        timing: ['durationMs'],
        outcome: ['status', 'changedKeys', 'error'],
        redaction: ['inputSummary', 'outputSummary'],
        guarantees: [
          'no_full_prompt_bodies',
          'no_private_memory_text',
          'no_api_keys_or_secrets',
        ],
      },
      totalRecords: safeRecords.length,
      records: safeRecords,
    },
    null,
    2,
  )
}

/**
 * 从 exportPromptHookAudit 的输出中解析回结构化数据。
 * 用于测试验证。
 *
 * @param {string} jsonStr
 * @returns {{ exportedAt: string, totalRecords: number, records: Array }}
 */
export function parsePromptHookAuditExport(jsonStr) {
  return JSON.parse(jsonStr)
}
