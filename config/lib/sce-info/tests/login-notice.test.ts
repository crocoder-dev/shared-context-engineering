import type { On } from 'claude-code'
import { expect, test } from 'claude-code/testing'
import { DOCTOR_ARGV, LOGIN_REQUIRED, VERSION_ARGV, VERSION_TIMEOUT_MS } from '../hooks/register'

const START = { cwd: '/repo', surface: 'terminal', isInteractive: true } as const

function answerDoctor(on: On, stdout: string, calls: unknown[] = []) {
  on('process.run', ($, e) => {
    calls.push(e)
    const output = e.argv[1] === '--version' ? 'shared-context-engineering 0.0.0\n' : stdout
    return { value: { exitCode: 0, stdout: output, stderr: '', isStdoutTruncated: false, isStderrTruncated: false } }
  })
}

function collectLog(on: On): string[] {
  const lines: string[] = []
  on('ui.log', ($, e) => {
    lines.push(e.text)
    return { value: undefined }
  })
  on('session.start', ($, e) => ({ cwd: e.cwd }))
  return lines
}

test('session start logs the login notice once when doctor reports not_logged_in', async ($, on) => {
  const calls: unknown[] = []
  answerDoctor(on, JSON.stringify({ sce_login: { state: 'not_logged_in', auth_mode: 'mandatory' } }), calls)
  const lines = collectLog(on)
  await $.session.start(START)
  expect(lines).toEqual([LOGIN_REQUIRED])
  expect(calls).toEqual([
    { argv: VERSION_ARGV, init: { cwd: '/repo', timeoutMs: VERSION_TIMEOUT_MS } },
    { argv: DOCTOR_ARGV, init: { cwd: '/repo' } },
  ])
})

test('session start logs nothing when already logged in', async ($, on) => {
  answerDoctor(on, JSON.stringify({ sce_login: { state: 'logged_in', auth_mode: 'mandatory' } }))
  const lines = collectLog(on)
  await $.session.start(START)
  expect(lines).toEqual([])
})

test('session start logs nothing when login is not mandatory', async ($, on) => {
  answerDoctor(on, JSON.stringify({ sce_login: { state: 'not_required', auth_mode: 'optional' } }))
  const lines = collectLog(on)
  await $.session.start(START)
  expect(lines).toEqual([])
})

test('session start logs nothing when the doctor command cannot run', async ($, on) => {
  on('process.run', ($, e) => {
    if (e.argv[1] === '--version') {
      return { value: { exitCode: 0, stdout: 'sce\n', stderr: '', isStdoutTruncated: false, isStderrTruncated: false } }
    }
    return { deny: 'doctor timed out' }
  })
  const lines = collectLog(on)
  await $.session.start(START)
  expect(lines).toEqual([])
})

test('session start logs nothing when doctor output is unusable', async ($, on) => {
  answerDoctor(on, 'not json')
  const lines = collectLog(on)
  await $.session.start(START)
  expect(lines).toEqual([])
})

test('non-interactive sessions skip the notice', async ($, on) => {
  const calls: unknown[] = []
  answerDoctor(on, JSON.stringify({ sce_login: { state: 'not_logged_in' } }), calls)
  const lines = collectLog(on)
  await $.session.start({ ...START, surface: null, isInteractive: false })
  expect(lines).toEqual([])
  expect(calls).toEqual([])
})
