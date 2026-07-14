/**
 * Machine-readable + Markdown compatibility report generators.
 *
 * Keeps supported / degraded / noop / intentionally_unsupported distinct and
 * refuses to claim full ST 99 or GUI acceptance.
 */

import {
  PLUGIN_COMPAT_MATRIX,
  SUPPORT,
  assertNonSupportedRowsHaveReasons,
  countByStatus,
  countBySurface,
  entriesRequiringExplicitBehavior,
} from './pluginCompatMatrix.js'

const CLAIM_GUARDS = Object.freeze([
  'not_full_st_99',
  'not_real_iframe_gui_acceptance',
  'classifications_remain_distinct',
])

/**
 * Generate a machine-readable compatibility report object.
 *
 * @param {Array<object>} [matrix]
 * @returns {object}
 */
export function generateMachineReadableReport(matrix = PLUGIN_COMPAT_MATRIX) {
  const statusCounts = countByStatus(matrix)
  const surfaceCounts = countBySurface(matrix)
  const reasonCheck = assertNonSupportedRowsHaveReasons(matrix)
  const remaining = entriesRequiringExplicitBehavior()
    .filter((entry) => matrix.includes(entry) || matrix.some((row) => row.id === entry.id))
    .map((entry) => ({
      id: entry.id,
      surface: entry.surface,
      name: entry.name,
      status: entry.status,
      reason: entry.reason || null,
      fallback: entry.fallback || null,
    }))

  return {
    kind: 'plugin_compat_report',
    schemaVersion: 1,
    generatedAt: new Date().toISOString(),
    claims: {
      fullSt99: false,
      realIframeGuiAcceptance: false,
      guards: CLAIM_GUARDS,
    },
    totals: {
      rows: matrix.length,
      byStatus: statusCounts,
      bySurface: surfaceCounts,
    },
    nonSupportedReasonCheck: reasonCheck,
    remainingDegradedOrUnsupported: remaining.filter((row) => (
      row.status === SUPPORT.DEGRADED
      || row.status === SUPPORT.INTENTIONALLY_UNSUPPORTED
      || row.status === SUPPORT.NOOP
    )),
    rows: matrix.map((entry) => ({
      id: entry.id,
      surface: entry.surface,
      name: entry.name,
      status: entry.status,
      reason: entry.reason || null,
      fallback: entry.fallback || null,
      mapsFrom: entry.mapsFrom || null,
      requiresPermissions: entry.requiresPermissions || [],
      failPolicy: entry.failPolicy || null,
    })),
  }
}

/**
 * Generate a Markdown summary of the compatibility report.
 *
 * @param {Array<object>} [matrix]
 * @returns {string}
 */
export function generateMarkdownReport(matrix = PLUGIN_COMPAT_MATRIX) {
  const report = generateMachineReadableReport(matrix)
  const lines = []
  lines.push('# Plugin Compatibility Report')
  lines.push('')
  lines.push(`- Generated: ${report.generatedAt}`)
  lines.push(`- Total rows: ${report.totals.rows}`)
  lines.push('- Full ST 99 claim: **no**')
  lines.push('- Real iframe/GUI acceptance claim: **no**')
  lines.push('')
  lines.push('## Counts by status')
  lines.push('')
  lines.push('| Status | Count |')
  lines.push('|--------|------:|')
  for (const [status, count] of Object.entries(report.totals.byStatus)) {
    lines.push(`| ${status} | ${count} |`)
  }
  lines.push('')
  lines.push('## Counts by surface')
  lines.push('')
  lines.push('| Surface | Count |')
  lines.push('|---------|------:|')
  for (const [surface, count] of Object.entries(report.totals.bySurface)) {
    lines.push(`| ${surface} | ${count} |`)
  }
  lines.push('')
  lines.push('## Remaining degraded / noop / unsupported')
  lines.push('')
  if (report.remainingDegradedOrUnsupported.length === 0) {
    lines.push('_none_')
  } else {
    lines.push('| Id | Status | Reason | Fallback |')
    lines.push('|----|--------|--------|----------|')
    for (const row of report.remainingDegradedOrUnsupported) {
      lines.push(`| ${row.id} | ${row.status} | ${row.reason || ''} | ${row.fallback || ''} |`)
    }
  }
  lines.push('')
  lines.push('## Guards')
  lines.push('')
  for (const guard of report.claims.guards) {
    lines.push(`- ${guard}`)
  }
  lines.push('')
  return lines.join('\n')
}

/**
 * Serialize the machine-readable report as pretty JSON.
 */
export function generateMachineReadableReportJson(matrix = PLUGIN_COMPAT_MATRIX) {
  return JSON.stringify(generateMachineReadableReport(matrix), null, 2)
}