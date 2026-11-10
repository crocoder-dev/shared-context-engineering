import type { EngineInterface, On, Timer } from 'claude-code'

let started = false
let missing = false
let pulseFrame = 0
let pulseTimer: Timer | undefined
let refreshTimer: Timer | undefined
let refreshing = false
let sessionCwd: string | undefined

export const ART = [
  '███████╗ ██████╗███████╗',
  '██╔════╝██╔════╝██╔════╝',
  '███████╗██║     █████╗',
  '╚════██║██║     ██╔══╝',
  '███████║╚██████╗███████╗',
  '╚══════╝ ╚═════╝╚══════╝',
]

const GRADIENT_LEFT = [255, 0, 255] as const
const GRADIENT_RIGHT = [0, 255, 255] as const
const ART_WIDTH = Math.max(...ART.map((row) => [...row].length))

export function gradientColor(col: number): string {
  const t = col / Math.max(ART_WIDTH - 1, 1)
  return (
    '#' +
    GRADIENT_LEFT.map((a, i) => Math.round(a + ((GRADIENT_RIGHT[i] ?? a) - a) * t).toString(16).padStart(2, '0')).join('')
  )
}

export const PULSE = ['#ff3131', '#e02b2b', '#b82323', '#8f1b1b', '#b82323', '#e02b2b'] as const
export const PULSE_INTERVAL_MS = 150

export const EXPECTED_SCE_VERSION = '__SCE_VERSION__'
export const SITE = 'sce.crocoder.dev'
export const INSTALL_URL = `https://${SITE}/docs/getting-started#install-cli`
export const FOOTER = `Visit ${SITE} to learn more.`
export const LOADED = 'loaded.'
export const NOT_LOADED = 'not loaded.'
export const INSTALL_FOOTER = `Install sce ${EXPECTED_SCE_VERSION}: `
export const SCE_MISSING = `CLI not found. Install sce ${EXPECTED_SCE_VERSION} from ${INSTALL_URL}`
export const LOGIN_REQUIRED = "Login required for this repository. Run 'sce auth login'."
export const VERSION_ARGV = ['sce', '--version'] as const
export const VERSION_TIMEOUT_MS = 5000
export const DOCTOR_ARGV = ['sce', 'doctor', '--format', 'json'] as const
export const STATUS_REFRESH_INTERVAL_MS = 10 * 60 * 1000
export const REFRESH_COMMAND = /\b(sce|git)\b/
export const GREEN = '🟢'
export const YELLOW = '🟡'
export const RED = '🔴'
export const WHITE = '⚪'
export const GIT_HOOKS_NOT_SET_UP = `${RED} git hooks not set up, run \`sce setup --hooks\``
export const ISSUES_HINT = 'run `sce doctor`'
export const STATUS_MISSING = `${RED} not installed`

type DoctorProblem = {
  kind?: string
  target?: string | null
  summary?: string
  remediation?: { text?: string }
}

type DoctorHook = {
  name?: string
  exists?: boolean
  executable?: boolean
  content_state?: string
}

type DoctorReport = {
  sce_login?: { state?: string }
  post_commit_auto_sync?: { enabled?: boolean; state?: string }
  hooks?: DoctorHook[]
  problems?: DoctorProblem[]
}

export const CLAUDE_ASSETS_STALE = "Claude SCE skills are outdated or modified. Run 'sce setup --claude' to update them."

const CLAUDE_ASSET_KINDS = new Set([
  'claude_integration_files_missing',
  'claude_integration_content_mismatch',
  'claude_asset_read_failed',
])

export function gitHooksMissing(names: readonly string[]): string {
  return `Git hooks missing: ${names.join(', ')}. Run 'sce doctor --fix' to install them.`
}

function parseDoctor(doctorJson: string): DoctorReport | undefined {
  try {
    const report = JSON.parse(doctorJson)
    return report && typeof report === 'object' ? report : undefined
  } catch {
    return undefined
  }
}

export function loginRequired(doctorJson: string): boolean {
  return parseDoctor(doctorJson)?.sce_login?.state === 'not_logged_in'
}

function missingGitHookNames(report: DoctorReport): string[] {
  if (!Array.isArray(report.hooks)) return []
  return report.hooks
    .filter((hook) => hook && hook.exists === false && typeof hook.name === 'string')
    .map((hook) => hook.name as string)
}

function gitHooksSetUp(report: DoctorReport): boolean {
  if (!Array.isArray(report.hooks)) return true
  return report.hooks.every((hook) => hook && hook.exists && hook.executable && hook.content_state === 'current')
}

export function problemNotices(doctorJson: string): string[] {
  const report = parseDoctor(doctorJson)
  if (!report) return []
  const missingHooks = missingGitHookNames(report)
  const hookNotices = missingHooks.length > 0 ? [gitHooksMissing(missingHooks)] : []
  return [...hookNotices, ...claudeNotices(report)]
}

function claudeNotices(report: DoctorReport): string[] {
  const claudeProblems = (Array.isArray(report.problems) ? report.problems : []).filter(
    (problem) => problem && problem.target === 'claude',
  )
  const assetNotices = claudeProblems.some((problem) => CLAUDE_ASSET_KINDS.has(problem.kind ?? ''))
    ? [CLAUDE_ASSETS_STALE]
    : []
  const otherNotices = claudeProblems
    .filter((problem) => !CLAUDE_ASSET_KINDS.has(problem.kind ?? '') && problem.summary)
    .map((problem) => [problem.summary, problem.remediation?.text].filter(Boolean).join(' '))
  return [...assetNotices, ...new Set(otherNotices)]
}

export function parseVersion(versionStdout: string): string | undefined {
  const [, version] = versionStdout.trim().split(/\s+/)
  return version || undefined
}

export const LOGIN_LABELS: Record<string, string> = {
  logged_in: `${GREEN} logged in`,
  not_logged_in: `${RED} not logged in, run \`sce auth login\``,
  unavailable: `${YELLOW} login status unavailable, run \`sce doctor\``,
  not_required: `${WHITE} login not required`,
}

export const AUTO_SYNC_LABELS: Record<string, string> = {
  ready: `${GREEN} auto sync on`,
  disabled: `${YELLOW} auto sync off`,
  not_ready: `${RED} auto sync not ready`,
  not_applicable: `${WHITE} auto sync n/a`,
}

export function shortVersion(version: string): string {
  return version.split('-')[0] ?? version
}

function installedLabel(version: string | undefined): string {
  if (!version) return `${GREEN} installed`
  if (version !== EXPECTED_SCE_VERSION) {
    return `${YELLOW} ${version} (expected ${EXPECTED_SCE_VERSION})`
  }
  return `${GREEN} ${shortVersion(version)}`
}

export function statusLine(versionStdout: string, doctorJson?: string): string {
  const parts = [installedLabel(parseVersion(versionStdout))]
  const report = doctorJson === undefined ? undefined : parseDoctor(doctorJson)
  if (doctorJson === undefined || !report) return parts.join(' ┃ ')
  const login = LOGIN_LABELS[report.sce_login?.state ?? '']
  if (login) parts.push(login)
  if (!gitHooksSetUp(report)) parts.push(GIT_HOOKS_NOT_SET_UP)
  const autoSync = AUTO_SYNC_LABELS[report.post_commit_auto_sync?.state ?? '']
  if (autoSync) parts.push(autoSync)
  const issues = claudeNotices(report).length
  if (issues > 0) parts.push(`${YELLOW} ${issues} issue${issues === 1 ? '' : 's'}, ${ISSUES_HINT}`)
  return parts.join(' ┃ ')
}

function startPulse($: EngineInterface) {
  if (pulseTimer || !missing || started) return
  pulseTimer = $.clock.every(PULSE_INTERVAL_MS, () => {
    pulseFrame = (pulseFrame + 1) % PULSE.length
    $.ui.invalidate('ui.render')
  })
}

function stopPulse() {
  pulseTimer?.cancel()
  pulseTimer = undefined
  pulseFrame = 0
}

async function refreshStatus($: EngineInterface, cwd: string) {
  if (refreshing) return
  refreshing = true
  try {
    const probe = await $.process.run(VERSION_ARGV, { cwd, timeoutMs: VERSION_TIMEOUT_MS }).catch(() => undefined)
    if (!probe) {
      $.ui.status(STATUS_MISSING)
      return
    }
    const doctor = await $.process.run(DOCTOR_ARGV, { cwd }).catch(() => undefined)
    $.ui.status(statusLine(probe.stdout, doctor?.stdout))
  } finally {
    refreshing = false
  }
}

function startRefresh($: EngineInterface, cwd: string) {
  refreshTimer?.cancel()
  refreshTimer = $.clock.every(STATUS_REFRESH_INTERVAL_MS, () => refreshStatus($, cwd))
}

export function register(on: On) {
  on('session.start', async ($, e, next) => {
    const result = await next(e)
    if (!e.isInteractive) return result
    sessionCwd = e.cwd
    startRefresh($, e.cwd)
    const probe = await $.process.run(VERSION_ARGV, { cwd: e.cwd, timeoutMs: VERSION_TIMEOUT_MS }).catch(() => undefined)
    if (!probe) {
      missing = true
      $.ui.status(STATUS_MISSING)
      startPulse($)
      $.ui.invalidate('ui.render')
      $.ui.log(SCE_MISSING)
      return result
    }
    if (missing) {
      missing = false
      stopPulse()
      $.ui.invalidate('ui.render')
    }
    $.ui.status(statusLine(probe.stdout))
    const doctor = await $.process.run(DOCTOR_ARGV, { cwd: e.cwd }).catch(() => undefined)
    if (!doctor) return result
    $.ui.status(statusLine(probe.stdout, doctor.stdout))
    if (loginRequired(doctor.stdout)) $.ui.log(LOGIN_REQUIRED)
    for (const notice of problemNotices(doctor.stdout)) $.ui.log(notice)
    return result
  })

  on('prompt.submit', async ($, e, next) => {
    if (!started) {
      started = true
      stopPulse()
      $.ui.invalidate('ui.render')
    }
    const result = await next(e)
    if (sessionCwd) await refreshStatus($, sessionCwd).catch(() => undefined)
    return result
  }).catch(($, e, next) => next(e))

  on('tool.call', { tool: 'Bash' }, async ($, e, next) => {
    const ran = await next(e)
    if (sessionCwd && REFRESH_COMMAND.test(e.command)) await refreshStatus($, sessionCwd)
    return ran
  })

  on('turn.complete', async ($, e, next) => {
    const result = await next(e)
    if (sessionCwd) await refreshStatus($, sessionCwd)
    return result
  })

  on('session.end', async ($, e, next) => {
    if (e.reason === 'clear' && started) {
      started = false
      startPulse($)
      $.ui.invalidate('ui.render')
    }
    return next(e)
  })

  on('ui.render', { component: 'AbovePrompt', surface: 'terminal' }, async ($, e, next) => {
    if (started || e.props.hasSurvey) return next(e)
    const { Box, Text, Link } = $.ui.resolve(e)
    const theirs = await next(e)
    const status = missing
      ? Text({ color: PULSE[pulseFrame], bold: true, children: ` ${NOT_LOADED}` })
      : Text({ color: 'cyan', children: ` ${LOADED}` })
    const footer = missing
      ? Text({
          children: [
            INSTALL_FOOTER,
            Text({ color: PULSE[pulseFrame], underline: true, children: Link({ href: INSTALL_URL, label: INSTALL_URL }) }),
          ],
        })
      : Text({
          children: [
            'Visit ',
            Text({ color: 'cyan', underline: true, children: Link({ href: `https://${SITE}`, label: SITE }) }),
            ' to learn more.',
          ],
        })
    const label = Box({
      paddingLeft: 11,
      flexDirection: 'column',
      children: [
        ...ART.map((row, i) => {
          const art = Text({ children: [...row].map((ch, col) => (ch === ' ' ? ch : Text({ color: gradientColor(col), children: ch }))) })
          if (i !== ART.length - 2) return art
          return Box({ flexDirection: 'row', children: [art, status] })
        }),
        footer,
      ],
    })
    return Box({ flexDirection: 'column', children: [label, theirs] })
  })
}
