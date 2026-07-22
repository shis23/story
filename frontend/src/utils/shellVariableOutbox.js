/**
 * Variable outbox for CardShell / TavernHelper writes.
 * Persists setvar/setChatVariable to campaign (default) or instance scope.
 */

/**
 * @param {unknown} value
 * @returns {import('../../tauri-api.js') extends never ? any : any}
 */
export function toJsonValue(value) {
  if (value === undefined) return null
  if (value === null) return null
  if (typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean') {
    return value
  }
  try {
    return JSON.parse(JSON.stringify(value))
  } catch {
    return String(value)
  }
}

/**
 * @param {object} opts
 * @param {string|null|undefined} opts.campaignId
 * @param {string|null|undefined} opts.instanceId
 * @param {string} opts.key
 * @param {unknown} opts.value
 * @param {(campaignId:string,key:string,value:any)=>Promise<any>} opts.setCampaignVariable
 * @param {(campaignId:string,instanceId:string,key:string,value:any)=>Promise<any>} opts.setCharacterVariable
 * @param {(level:string,message:string)=>Promise<any>|void} [opts.log]
 * @returns {Promise<{ok:boolean, scope:string, key:string, error?:string}>}
 */
export async function persistShellVariableWrite({
  campaignId,
  instanceId,
  key,
  value,
  setCampaignVariable,
  setCharacterVariable,
  log,
}) {
  const k = String(key || '').trim()
  if (!k) {
    return { ok: false, scope: 'none', key: '', error: 'empty key' }
  }
  if (!campaignId) {
    const err = 'no active campaign for variable write'
    if (log) await log('warn', `shell_var_write skip: ${err} key=${k}`)
    return { ok: false, scope: 'none', key: k, error: err }
  }

  const json = toJsonValue(value)
  // Prefix convention: instance:<id>:<key> or inst:<key> with explicit instanceId
  let scope = 'campaign'
  let writeKey = k
  let targetInstance = instanceId || null

  if (k.startsWith('instance:') || k.startsWith('inst:')) {
    const parts = k.split(':')
    if (parts.length >= 3) {
      targetInstance = parts[1]
      writeKey = parts.slice(2).join(':')
      scope = 'instance'
    } else if (parts.length === 2 && targetInstance) {
      writeKey = parts[1]
      scope = 'instance'
    }
  } else if (k.startsWith('campaign:')) {
    writeKey = k.slice('campaign:'.length)
    scope = 'campaign'
  }

  try {
    if (scope === 'instance') {
      if (!targetInstance) {
        const err = 'instance scope requires instanceId'
        if (log) await log('warn', `shell_var_write fail: ${err} key=${k}`)
        return { ok: false, scope, key: writeKey, error: err }
      }
      await setCharacterVariable(campaignId, targetInstance, writeKey, json)
    } else {
      await setCampaignVariable(campaignId, writeKey, json)
    }
    if (log) {
      await log(
        'info',
        `shell_var_write ok scope=${scope} campaign=${campaignId} instance=${targetInstance || '-'} key=${writeKey}`,
      )
    }
    return { ok: true, scope, key: writeKey }
  } catch (e) {
    const err = String(e?.message || e)
    if (log) await log('error', `shell_var_write fail scope=${scope} key=${writeKey}: ${err}`)
    return { ok: false, scope, key: writeKey, error: err }
  }
}

/**
 * Ring-buffer audit log of recent writes (for debug drawer / status strip).
 */
export function createVariableWriteAudit(limit = 30) {
  /** @type {Array<object>} */
  const items = []
  return {
    push(entry) {
      items.unshift({ ...entry, at: Date.now() })
      if (items.length > limit) items.length = limit
    },
    list() {
      return items.slice()
    },
    clear() {
      items.length = 0
    },
  }
}
