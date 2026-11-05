import type { On } from 'claude-code'
import { expect, mock, test } from 'claude-code/testing'
import {
  EXPECTED_SCE_VERSION,
  GIT_HOOKS_NOT_SET_UP,
  shortVersion,
  STATUS_MISSING,
  STATUS_REFRESH_INTERVAL_MS,
  statusLine,
} from '../hooks/register'

const START = { cwd: '/repo', surface: 'terminal', isInteractive: true } as const
const SCE = shortVersion(EXPECTED_SCE_VERSION)

function collectStatus(on: On, logs: string[] = []): (string | undefined)[] {
  const lines: (string | undefined)[] = []
  on('ui.status', ($, e) => {
    lines.push(e.text)
    return { value: undefined }
  })
  on('ui.log', ($, e) => {
    logs.push(e.text)
    return { value: undefined }
  })
  on('session.start', ($, e) => ({ cwd: e.cwd }))
  return lines
}

test('a missing sce CLI shows a red not installed status', async ($, on) => {
  on('process.run', () => ({ deny: 'spawn sce ENOENT' }))
  const lines = collectStatus(on)
  await $.session.start(START)
  expect(lines).toEqual([STATUS_MISSING])
  expect(STATUS_MISSING).toBe('🔴 not installed')
})

test('the status shows the version, then doctor findings', async ($, on) => {
  on('process.run', ($, e) => {
    const stdout =
      e.argv[1] === '--version'
        ? `shared-context-engineering ${EXPECTED_SCE_VERSION} (unknown)\n`
        : JSON.stringify({
            sce_login: { state: 'not_logged_in' },
            post_commit_auto_sync: { enabled: true, state: 'ready' },
            hooks: [{ name: 'pre-commit', exists: false }],
            problems: [{ kind: 'claude_integration_content_mismatch', target: 'claude' }],
          })
    return { value: { exitCode: 0, stdout, stderr: '', isStdoutTruncated: false, isStderrTruncated: false } }
  })
  const lines = collectStatus(on)
  await $.session.start(START)
  expect(lines).toEqual([`🟢 ${SCE}`, `🟢 ${SCE} ┃ 🔴 not logged in, run \`sce auth login\` ┃ 🔴 git hooks not set up, run \`sce setup --hooks\` ┃ 🟢 auto sync on ┃ 🟡 1 issue, run \`sce doctor\``])
})

test('a version mismatch is yellow and names the expected version', () => {
  expect(statusLine('shared-context-engineering 0.1.0\n', '{}')).toBe(`🟡 0.1.0 (expected ${EXPECTED_SCE_VERSION})`)
  expect(statusLine('sce\n')).toBe('🟢 installed')
})

test('a matching version drops its pre-release suffix', () => {
  expect(shortVersion('0.4.0-pre-alpha-v13')).toBe('0.4.0')
  expect(statusLine(`shared-context-engineering ${EXPECTED_SCE_VERSION}\n`)).toBe(`🟢 ${SCE}`)
})

test('login and auto sync states each get a colored label', () => {
  const line = (login: string, autoSync: string) =>
    statusLine(
      `shared-context-engineering ${EXPECTED_SCE_VERSION}\n`,
      JSON.stringify({ sce_login: { state: login }, post_commit_auto_sync: { state: autoSync } }),
    )
  expect(line('logged_in', 'ready')).toBe(`🟢 ${SCE} ┃ 🟢 logged in ┃ 🟢 auto sync on`)
  expect(line('not_logged_in', 'disabled')).toBe(`🟢 ${SCE} ┃ 🔴 not logged in, run \`sce auth login\` ┃ 🟡 auto sync off`)
  expect(line('not_required', 'not_ready')).toBe(`🟢 ${SCE} ┃ ⚪ login not required ┃ 🔴 auto sync not ready`)
  expect(line('logged_in', 'not_applicable')).toBe(`🟢 ${SCE} ┃ 🟢 logged in ┃ ⚪ auto sync n/a`)
})

test('git hooks show a setup hint unless every hook is installed and current', () => {
  const hook = (overrides: object) => ({ name: 'post-commit', exists: true, executable: true, content_state: 'current', ...overrides })
  const line = (hooks: object[]) =>
    statusLine(`shared-context-engineering ${EXPECTED_SCE_VERSION}\n`, JSON.stringify({ hooks }))
  expect(line([hook({})])).toBe(`🟢 ${SCE}`)
  expect(line([hook({ exists: false })])).toBe(`🟢 ${SCE} ┃ ${GIT_HOOKS_NOT_SET_UP}`)
  expect(line([hook({ executable: false })])).toBe(`🟢 ${SCE} ┃ ${GIT_HOOKS_NOT_SET_UP}`)
  expect(line([hook({ content_state: 'stale' })])).toBe(`🟢 ${SCE} ┃ ${GIT_HOOKS_NOT_SET_UP}`)
  expect(GIT_HOOKS_NOT_SET_UP).toBe('🔴 git hooks not set up, run `sce setup --hooks`')
})

test('the status refreshes every few minutes without logging again', async ($, on) => {
  const clock = mock.clock(on)
  let login = 'not_logged_in'
  const logs: string[] = []
  on('process.run', ($, e) => {
    const stdout =
      e.argv[1] === '--version'
        ? `shared-context-engineering ${EXPECTED_SCE_VERSION}\n`
        : JSON.stringify({ sce_login: { state: login } })
    return { value: { exitCode: 0, stdout, stderr: '', isStdoutTruncated: false, isStderrTruncated: false } }
  })
  const lines = collectStatus(on, logs)
  await $.session.start(START)
  expect(lines.at(-1)).toBe(`🟢 ${SCE} ┃ 🔴 not logged in, run \`sce auth login\``)
  const logged = logs.length

  login = 'logged_in'
  await clock.advance(STATUS_REFRESH_INTERVAL_MS)
  expect(lines.at(-1)).toBe(`🟢 ${SCE} ┃ 🟢 logged in`)
  expect(logs.length).toBe(logged)
})

test('an sce or git command refreshes the status; other commands do not', async ($, on) => {
  let login = 'not_logged_in'
  let doctorRuns = 0
  on('process.run', ($, e) => {
    if (e.argv[1] !== '--version') doctorRuns += 1
    const stdout =
      e.argv[1] === '--version'
        ? `shared-context-engineering ${EXPECTED_SCE_VERSION}\n`
        : JSON.stringify({ sce_login: { state: login } })
    return { value: { exitCode: 0, stdout, stderr: '', isStdoutTruncated: false, isStderrTruncated: false } }
  })
  on('tool.call', () => ({ result: {} }))
  const lines = collectStatus(on)
  await $.session.start(START)
  const runs = doctorRuns

  await $.tool.call({ tool: 'Bash', command: 'ls' })
  expect(doctorRuns).toBe(runs)

  login = 'logged_in'
  await $.tool.call({ tool: 'Bash', command: 'sce auth login' })
  expect(doctorRuns).toBe(runs + 1)
  expect(lines.at(-1)).toBe(`🟢 ${SCE} ┃ 🟢 logged in`)
})

test('sending a prompt refreshes the status', async ($, on) => {
  let login = 'logged_in'
  on('process.run', ($, e) => {
    const stdout =
      e.argv[1] === '--version'
        ? `shared-context-engineering ${EXPECTED_SCE_VERSION}\n`
        : JSON.stringify({ sce_login: { state: login } })
    return { value: { exitCode: 0, stdout, stderr: '', isStdoutTruncated: false, isStderrTruncated: false } }
  })
  mock.clock(on)
  on('prompt.submit', ($, e) => ({ text: e.text }))
  const lines = collectStatus(on)
  await $.session.start(START)
  expect(lines.at(-1)).toBe(`🟢 ${SCE} ┃ 🟢 logged in`)

  login = 'not_logged_in'
  await $.prompt.submit({ text: 'hello', wait: false })
  expect(lines.at(-1)).toBe(`🟢 ${SCE} ┃ 🔴 not logged in, run \`sce auth login\``)
})
