import assert from 'node:assert/strict'
import test from 'node:test'

import {
  PLUGIN_COMPAT_MATRIX,
  SUPPORT,
  assertNonSupportedRowsHaveReasons,
  countByStatus,
} from '../src/utils/pluginCompatMatrix.js'
import {
  generateMachineReadableReport,
  generateMachineReadableReportJson,
  generateMarkdownReport,
} from '../src/utils/pluginCompatReport.js'

test('every non-supported matrix row has an explicit reason', () => {
  const check = assertNonSupportedRowsHaveReasons()
  assert.equal(check.ok, true, `missing reasons: ${check.missing.join(', ')}`)
  assert.ok(check.total > 0)
})

test('machine-readable report keeps classifications distinct and refuses ST 99 claims', () => {
  const report = generateMachineReadableReport()
  assert.equal(report.kind, 'plugin_compat_report')
  assert.equal(report.schemaVersion, 1)
  assert.equal(report.claims.fullSt99, false)
  assert.equal(report.claims.realIframeGuiAcceptance, false)
  assert.ok(report.claims.guards.includes('not_full_st_99'))
  assert.ok(report.totals.rows >= PLUGIN_COMPAT_MATRIX.length)
  assert.ok(report.totals.byStatus[SUPPORT.IMPLEMENTED] > 0)
  assert.ok(report.totals.byStatus[SUPPORT.DEGRADED] > 0)
  assert.ok(report.totals.byStatus[SUPPORT.INTENTIONALLY_UNSUPPORTED] > 0)
  assert.ok(report.totals.byStatus[SUPPORT.NOOP] > 0)
  // Distinct: no status is collapsed into another.
  const statuses = new Set(report.rows.map((row) => row.status))
  assert.ok(statuses.has(SUPPORT.IMPLEMENTED))
  assert.ok(statuses.has(SUPPORT.DEGRADED))
  assert.ok(statuses.has(SUPPORT.INTENTIONALLY_UNSUPPORTED))
  assert.ok(statuses.has(SUPPORT.NOOP))
  assert.equal(report.nonSupportedReasonCheck.ok, true)
})

test('markdown report summarizes counts and remaining degraded/unsupported rows', () => {
  const md = generateMarkdownReport()
  assert.match(md, /# Plugin Compatibility Report/)
  assert.match(md, /Full ST 99 claim: \*\*no\*\*/)
  assert.match(md, /Real iframe\/GUI acceptance claim: \*\*no\*\*/)
  assert.match(md, /## Counts by status/)
  assert.match(md, /## Remaining degraded \/ noop \/ unsupported/)
  assert.match(md, /th:saveChat/)
  assert.match(md, /degraded/)
})

test('machine-readable JSON is parseable and includes remaining degraded rows', () => {
  const json = generateMachineReadableReportJson()
  const parsed = JSON.parse(json)
  assert.equal(parsed.kind, 'plugin_compat_report')
  assert.ok(Array.isArray(parsed.remainingDegradedOrUnsupported))
  assert.ok(parsed.remainingDegradedOrUnsupported.some((row) => row.id === 'th:saveChat'))
  assert.ok(parsed.remainingDegradedOrUnsupported.some((row) => row.status === SUPPORT.INTENTIONALLY_UNSUPPORTED))
})

test('countByStatus matches report totals', () => {
  const counts = countByStatus()
  const report = generateMachineReadableReport()
  assert.deepEqual(report.totals.byStatus, counts)
})

test('report rows preserve failPolicy metadata when present', () => {
  const report = generateMachineReadableReport()
  const cancel = report.rows.find((row) => row.id === 'hook:cancel')
  assert.ok(cancel)
  assert.equal(cancel.failPolicy, 'fail_closed')
  const timeout = report.rows.find((row) => row.id === 'hook:timeout')
  assert.equal(timeout.failPolicy, 'fail_open')
})
