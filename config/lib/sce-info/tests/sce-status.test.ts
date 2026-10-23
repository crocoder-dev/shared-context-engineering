import type { On } from 'claude-code'
import { expect, mock, test } from 'claude-code/testing'
import {
  ART,
  CLAUDE_ASSETS_STALE,
  gitHooksMissing,
  INSTALL_URL,
  LOADED,
  NOT_LOADED,
  PULSE,
  PULSE_INTERVAL_MS,
  SCE_MISSING,
  SITE,
} from '../hooks/register'


const START = { cwd: '/repo', surface: 'terminal', isInteractive: true } as const

const BAND = {
  plugin: 'SCE',
  surface: 'terminal',
  component: 'AbovePrompt',
  props: {
    hasSurvey: false,
    isWorking: false,
    maxRows: 20,
    bodyColumns: 100,
    scroll: { offset: 0, bodyRows: 20 },
    view: {},
  },
} as const

function sceMissing(on: On, calls: string[][] = []) {
  on('process.run', ($, e) => {
    calls.push([...e.argv])
    return { deny: 'spawn sce ENOENT' }
  })
}

function doctorReports(on: On, report: unknown) {
  on('process.run', ($, e) => {
    const stdout = e.argv[1] === '--version' ? 'sce\n' : JSON.stringify(report)
    return { value: { exitCode: 0, stdout, stderr: '', isStdoutTruncated: false, isStderrTruncated: false } }
  })
}

function collectLog(on: On): string[] {
  const lines: string[] = []
  on('ui.log', ($, e) => {
    lines.push(e.text)
    return { value: undefined }
  })
  on('session.start', ($, e) => ({ cwd: e.cwd }))
  on('ui.render', { component: 'AbovePrompt' }, ($, e) => {
    const { Text } = $.ui.resolve(e)
    return Text({ children: 'beneath' })
  })
  return lines
}

test('a missing sce CLI logs the install guidance and skips doctor', async ($, on) => {
  const clock = mock.clock(on)
  const calls: string[][] = []
  sceMissing(on, calls)
  const lines = collectLog(on)
  await $.session.start(START)
  expect(lines).toEqual([SCE_MISSING])
  expect(SCE_MISSING).toContain('https://sce.crocoder.dev/docs/getting-started#install-cli')
  expect(calls).toEqual([['sce', '--version']])
  await clock.advance(0)
})

test('a missing sce CLI turns the splash into a pulsing red "not loaded."', async ($, on) => {
  const clock = mock.clock(on)
  sceMissing(on)
  collectLog(on)
  await $.session.start(START)
  const ui = await $.ui.mount(BAND)

  expect(await ui.find({ type: 'Text', text: ART[0] })).toBeDefined()
  const loaded = await ui.findAll({ type: 'Text', text: ` ${LOADED}` })
  expect(loaded.map((node) => node.text)).toEqual([` ${NOT_LOADED}`])
  const links = await ui.findAll({ type: 'Link', text: SITE })
  expect(links.map((node) => node.props.href)).toEqual([INSTALL_URL])

  const status = await ui.find({ type: 'Text', text: ` ${NOT_LOADED}` })
  expect(status?.props.color).toBe(PULSE[0])

  await clock.advance(PULSE_INTERVAL_MS)
  await ui.redraw()
  expect((await ui.find({ type: 'Text', text: ` ${NOT_LOADED}` }))?.props.color).toBe(PULSE[1])

  await clock.advance(PULSE_INTERVAL_MS * (PULSE.length - 1))
  await ui.redraw()
  expect((await ui.find({ type: 'Text', text: ` ${NOT_LOADED}` }))?.props.color).toBe(PULSE[0])
})

test('the pulse stops once the first prompt is sent', async ($, on) => {
  const clock = mock.clock(on)
  sceMissing(on)
  collectLog(on)
  on('prompt.submit', ($, e) => ({ text: e.text }))
  await $.session.start(START)
  const ui = await $.ui.mount(BAND)
  await $.prompt.submit({ text: 'hello', wait: false, origin: { kind: 'composer' } })
  await clock.advance(PULSE_INTERVAL_MS * 3)
  expect(await ui.find({ type: 'Text', text: ` ${NOT_LOADED}` })).toBeUndefined()
  expect(await ui.find({ type: 'Text', text: 'beneath' })).toBeDefined()
})

const HOOK_PROBLEMS = ['pre-commit', 'commit-msg', 'post-commit'].map((name) => ({
  kind: 'required_hook_missing',
  target: null,
  category: 'hook_rollout',
  summary: `Missing required hook '${name}'.`,
  remediation: { next_action: 'doctor_fix', text: "Run 'sce doctor --fix'." },
}))

test('missing git hooks collapse into one notice listing every missing hook', async ($, on) => {
  doctorReports(on, {
    sce_login: { state: 'logged_in' },
    hooks: [
      { name: 'pre-commit', exists: false, state: 'missing' },
      { name: 'commit-msg', exists: true, state: 'ok' },
      { name: 'post-commit', exists: false, state: 'missing' },
    ],
    problems: [HOOK_PROBLEMS[0], HOOK_PROBLEMS[2]],
  })
  const lines = collectLog(on)
  await $.session.start(START)
  expect(lines).toEqual([gitHooksMissing(['pre-commit', 'post-commit'])])
  expect(lines[0]).toBe("Git hooks missing: pre-commit, post-commit. Run 'sce doctor --fix' to install them.")
})

test('git hooks that all exist produce no git hook notice', async ($, on) => {
  doctorReports(on, {
    sce_login: { state: 'logged_in' },
    hooks: [
      { name: 'pre-commit', exists: true, state: 'ok' },
      { name: 'commit-msg', exists: true, state: 'ok' },
      { name: 'post-commit', exists: true, state: 'ok' },
    ],
    problems: [],
  })
  const lines = collectLog(on)
  await $.session.start(START)
  expect(lines).toEqual([])
})

test('doctor problems for Claude are shown; other targets are not', async ($, on) => {
  doctorReports(on, {
    sce_login: { state: 'logged_in' },
    hooks: [
      { name: 'pre-commit', exists: false, state: 'missing' },
      { name: 'commit-msg', exists: false, state: 'missing' },
      { name: 'post-commit', exists: false, state: 'missing' },
    ],
    problems: [
      ...HOOK_PROBLEMS,
      {
        kind: 'claude_integration_files_missing',
        target: 'claude',
        category: 'repo_assets',
        summary: 'ClaudeCode skills required file(s) are missing.',
        remediation: { next_action: 'manual_steps', text: 'Reinstall repo-root Claude assets.' },
      },
      {
        kind: 'claude_integration_content_mismatch',
        target: 'claude',
        category: 'repo_assets',
        summary: 'ClaudeCode commands file(s) differ from the canonical embedded content.',
        remediation: { next_action: 'manual_steps', text: 'Reinstall repo-root Claude assets.' },
      },
      {
        kind: 'mutation_scope_health_blocked',
        target: 'claude',
        category: 'mutation_scope_health',
        summary: 'Claude mutation scope is blocked.',
        remediation: { next_action: 'doctor_fix', text: "Run 'sce doctor --fix'." },
      },
      {
        kind: 'codex_hook_registration_not_trusted',
        target: 'codex',
        category: 'repo_assets',
        summary: 'Codex has not trusted the hooks.',
        remediation: { next_action: 'manual_steps', text: 'Trust them in Codex.' },
      },
      {
        kind: 'sce_login_required',
        target: null,
        category: 'global_state',
        summary: 'No valid SCE login was found.',
        remediation: { next_action: 'auth_login', text: "Run 'sce auth login'." },
      },
    ],
  })
  const lines = collectLog(on)
  await $.session.start(START)
  expect(lines).toEqual([
    gitHooksMissing(['pre-commit', 'commit-msg', 'post-commit']),
    CLAUDE_ASSETS_STALE,
    "Claude mutation scope is blocked. Run 'sce doctor --fix'.",
  ])
  expect(CLAUDE_ASSETS_STALE).toBe("Claude SCE skills are outdated or modified. Run 'sce setup --claude' to update them.")
})

test('a healthy doctor report keeps the splash "loaded." and logs nothing', async ($, on) => {
  doctorReports(on, { sce_login: { state: 'logged_in' }, problems: [] })
  const lines = collectLog(on)
  await $.session.start(START)
  const ui = await $.ui.mount(BAND)
  expect(lines).toEqual([])
  expect(await ui.findAll({ type: 'Text', text: ` ${LOADED}` })).toHaveLength(1)
  expect(await ui.find({ type: 'Text', text: ` ${NOT_LOADED}` })).toBeUndefined()
})
