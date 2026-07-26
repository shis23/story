/**
 * An opening shell is an initial-setup surface, not part of the permanent
 * reading flow. It may coexist with the selected greeting, but never with an
 * established conversation or an in-progress turn.
 */
export function shouldShowOpeningShell({
  openingUrl,
  openingArmed = false,
  messageCount = 0,
  isWriting = false,
} = {}) {
  return Boolean(openingUrl) && openingArmed && messageCount <= 1 && !isWriting
}

/**
 * A setup-only opening replaces EmptyHero. Fresh Campaigns create their
 * conversation before the user chooses an opening, so an empty message list
 * alone must not send that flow to the generic blank homepage.
 */
export function shouldShowEmptyWritingState({
  showOpening = false,
  messageCount = 0,
  isWriting = false,
  greetingCount = 0,
} = {}) {
  return !showOpening && messageCount === 0 && !isWriting && greetingCount === 0
}

/**
 * A Campaign owns its card-shell surfaces. Never let a previously selected
 * legacy character override a Campaign's card when choosing the opening or
 * status shell.
 */
export function resolveCardShellManifestTarget({
  activeCampaign = null,
  activeChar = null,
  activeCharDetail = null,
} = {}) {
  if (activeCampaign) {
    const cardId = activeCampaign.card_id || null
    return cardId ? { kind: 'campaign-card', cardId } : null
  }
  const characterId =
    activeChar?.id ||
    activeCharDetail?.id ||
    activeCharDetail?.source_character_id ||
    null
  return characterId ? { kind: 'character', characterId } : null
}

/**
 * Setup pages frequently contain their own scroller. Their document height is
 * therefore not a reliable indication of the reading space they need. Keep
 * the opening on screen as a viewport-sized surface and let that page scroll
 * internally instead of shrinking the iframe to its first small container.
 *
 * Prefer a concrete pixel height: WebView2 has historically dropped compound
 * `min()`/`calc()` height values on sandboxed iframes, which left the opening
 * at the 120px min-height and looked like a "tiny strip".
 */
export function getOpeningShellPresentation(viewportHeight = defaultViewportHeight()) {
  const raw = Number(viewportHeight)
  const vh = Number.isFinite(raw) && raw > 0 ? raw : 900
  // Leave room for top bar + composer while still filling most of the reading
  // surface. Floor keeps multi-step home pages usable on short windows.
  const heightPx = Math.max(520, Math.min(900, Math.round(vh - 160)))
  return {
    height: `${heightPx}px`,
    autoHeight: false,
  }
}

function defaultViewportHeight() {
  try {
    if (typeof window !== 'undefined' && window.innerHeight) return window.innerHeight
  } catch (_) {
    /* ignore */
  }
  return 900
}

/** Keep automatic shell height safe without turning it back into a tiny pane. */
export function normalizeShellHeight(value, { min = 160, max = 10_000 } = {}) {
  const height = Math.ceil(Number(value))
  if (!Number.isFinite(height) || height <= 0) return null
  return Math.min(Math.max(height, min), max)
}
