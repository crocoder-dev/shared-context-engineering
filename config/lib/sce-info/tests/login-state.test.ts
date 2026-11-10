import { expect, test } from 'bun:test'
import { EXPECTED_SCE_VERSION, loginRequired, shortVersion, statusLine } from '../hooks/register'

const version = `shared-context-engineering ${EXPECTED_SCE_VERSION}\n`
const prefix = `🟢 ${shortVersion(EXPECTED_SCE_VERSION)}`

test('unavailable login status is yellow and never asks for login', () => {
  for (const reason of ['credential_inaccessible', 'database_locked', 'database_error', 'path_resolution_error', 'schema_not_current', 'lookup_timeout', 'clock_error']) {
    const doctor = JSON.stringify({ sce_login: { state: 'unavailable', reason } })
    expect(statusLine(version, doctor)).toBe(`${prefix} ┃ 🟡 login status unavailable, run \`sce doctor\``)
    expect(loginRequired(doctor)).toBe(false)
  }
})

test('telemetry readiness cannot override a saved login', () => {
  for (const state of ['not_configured', 'disabled', 'disabled_by_config']) {
    const doctor = JSON.stringify({ sce_login: { state: 'logged_in' }, managed_telemetry: { state } })
    expect(statusLine(version, doctor)).toBe(`${prefix} ┃ 🟢 logged in`)
    expect(loginRequired(doctor)).toBe(false)
  }
})

test('a genuinely missing login still requests authentication', () => {
  const doctor = JSON.stringify({ sce_login: { state: 'not_logged_in' } })
  expect(statusLine(version, doctor)).toBe(`${prefix} ┃ 🔴 not logged in, run \`sce auth login\``)
  expect(loginRequired(doctor)).toBe(true)
})
