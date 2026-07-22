function entryName(entry) {
  const name = String(entry?.name || '').trim()
  if (name) return name

  const firstKey = Array.isArray(entry?.keys)
    ? entry.keys.find((key) => String(key || '').trim())
    : null
  if (firstKey) return String(firstKey).trim()

  return `world-info-${Number(entry?.index) || 0}`
}

/**
 * Convert the active Campaign world-info DTO into the entry shape consumed by
 * TavernHelper's getWorldbook API. Campaign storage remains authoritative;
 * this only exposes its title and enabled state to the isolated card shell.
 *
 * @param {{entries?: Array<object>} | null | undefined} worldbook
 * @returns {Array<{index: number, name: string, enabled: boolean}>}
 */
export function mapCampaignWorldbookForTavernHelper(worldbook) {
  return Array.isArray(worldbook?.entries)
    ? worldbook.entries.map((entry) => ({
      index: Number(entry?.index) || 0,
      name: entryName(entry),
      enabled: !entry?.disabled,
    }))
    : []
}

/**
 * Determine which persisted Campaign entries need their enabled state changed
 * after a TavernHelper updateWorldbookWith callback.
 *
 * @param {Array<object>} currentEntries
 * @param {Array<{name?: string, enabled?: boolean}>} requestedEntries
 * @returns {Array<{entryIndex: number, enabled: boolean}>}
 */
export function resolveCampaignWorldbookEnabledUpdates(currentEntries, requestedEntries) {
  const requestedByName = new Map(
    (Array.isArray(requestedEntries) ? requestedEntries : [])
      .filter((entry) => entry?.name != null)
      .map((entry) => [String(entry.name), Boolean(entry.enabled)]),
  )

  return (Array.isArray(currentEntries) ? currentEntries : [])
    .flatMap((entry) => {
      const requestedEnabled = requestedByName.get(entryName(entry))
      if (requestedEnabled === undefined || requestedEnabled === !entry?.disabled) return []
      return [{ entryIndex: Number(entry?.index) || 0, enabled: requestedEnabled }]
    })
}

/**
 * Persist changes one at a time. Campaign world-info storage updates a shared
 * book, so concurrent writes can otherwise lose a sibling DLC/core toggle.
 *
 * @param {Array<{entryIndex: number, enabled: boolean}>} updates
 * @param {(update: {entryIndex: number, enabled: boolean}) => Promise<unknown>} persist
 * @returns {Promise<void>}
 */
export async function applyCampaignWorldbookEnabledUpdates(updates, persist) {
  for (const update of Array.isArray(updates) ? updates : []) {
    await persist(update)
  }
}

// Visible status, opening, and message shells share the same Campaign book.
// Keep their independent bridge callbacks ordered in this renderer process so
// a second host cannot race a read-modify-write batch from the first one.
const campaignWriteTails = new Map()

/**
 * Run a complete Campaign worldbook mutation behind earlier mutations for
 * that Campaign. Reading the book and deciding which entries changed must be
 * inside this queue too; otherwise a callback can act on a stale snapshot even
 * when the eventual persistence calls themselves are ordered.
 *
 * @template T
 * @param {string | null | undefined} campaignId
 * @param {() => Promise<T>} mutation
 * @returns {Promise<T>}
 */
export function enqueueCampaignWorldbookMutation(campaignId, mutation) {
  const key = String(campaignId || '')
  if (!key) return mutation()

  const previous = campaignWriteTails.get(key) || Promise.resolve()
  const task = previous
    .catch(() => undefined)
    .then(mutation)
  const tail = task.finally(() => {
    if (campaignWriteTails.get(key) === tail) campaignWriteTails.delete(key)
  })
  campaignWriteTails.set(key, tail)
  return task
}

/**
 * Queue a precomputed batch of enabled-state updates. Prefer
 * enqueueCampaignWorldbookMutation when the current book still needs to be
 * read, so snapshot resolution is queued together with persistence.
 *
 * @param {string | null | undefined} campaignId
 * @param {Array<{entryIndex: number, enabled: boolean}>} updates
 * @param {(update: {entryIndex: number, enabled: boolean}) => Promise<unknown>} persist
 * @returns {Promise<void>}
 */
export function enqueueCampaignWorldbookEnabledUpdates(campaignId, updates, persist) {
  return enqueueCampaignWorldbookMutation(campaignId, () =>
    applyCampaignWorldbookEnabledUpdates(updates, persist),
  )
}
