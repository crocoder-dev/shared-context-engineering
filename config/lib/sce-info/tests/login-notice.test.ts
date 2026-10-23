import { expect, test } from 'claude-code/testing'
import { DOCTOR_ARGV, LOGIN_REQUIRED } from '../hooks/register'

const START = { cwd: '/repo', surface: 'terminal', isInteractive: true } as const

function answerDoctor(on: Parameters<Parameters<typeof test>[1]>[1], stdout: string, calls: unknown[] = []) {
  on('process.run', ($, e) => {
    calls.push(e)
    return { value: { exitCode: 0, stdout, stderr: '', isStdoutTruncated: false, isStderrTruncated: false } }
  })
}

function collectLog(on: Parameters<Parameters<typeof test>[1]>[1]): string[] {
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
  expect(calls).toEqual([{ argv: DOCTOR_ARGV, init: { cwd: '/repo' } }])
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
  on('process.run', () => ({ deny: 'sce not found' }))
  const lines = collectLog(on)
  await $.session.start(START)
  expect(lines).toEqual([])
})

test('session start logs nothing when doctor output is unusable', async ($, on) => {
  answerDoctor(on, 'sce CLI not found')
  const lines = collectLog(on)
  await $.session.start(START)
  expect(lines).toEqual([])
})

test('non-interactive sessions skip the notice', async ($, on) => {
  answerDoctor(on, JSON.stringify({ sce_login: { state: 'not_logged_in' } }))
  const lines = collectLog(on)
  await $.session.start({ ...START, surface: null, isInteractive: false })
  expect(lines).toEqual([])
})
