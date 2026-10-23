import type { EngineInterface, On, Timer } from 'claude-code'

let started = false
let missing = false
let pulseFrame = 0
let pulseTimer: Timer | undefined

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

export const PULSE = ['#ff2d2d', '#e02424', '#b81c1c', '#8f1414', '#b81c1c', '#e02424'] as const
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

type DoctorProblem = {
  kind?: string
  target?: string | null
  summary?: string
  remediation?: { text?: string }
}

type DoctorHook = {
  name?: string
  exists?: boolean
}

type DoctorReport = {
  sce_login?: { state?: string }
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

export function problemNotices(doctorJson: string): string[] {
  const report = parseDoctor(doctorJson)
  if (!report) return []
  const missingHooks = missingGitHookNames(report)
  const hookNotices = missingHooks.length > 0 ? [gitHooksMissing(missingHooks)] : []
  const claudeProblems = (Array.isArray(report.problems) ? report.problems : []).filter(
    (problem) => problem && problem.target === 'claude',
  )
  const assetNotices = claudeProblems.some((problem) => CLAUDE_ASSET_KINDS.has(problem.kind ?? ''))
    ? [CLAUDE_ASSETS_STALE]
    : []
  const otherNotices = claudeProblems
    .filter((problem) => !CLAUDE_ASSET_KINDS.has(problem.kind ?? '') && problem.summary)
    .map((problem) => [problem.summary, problem.remediation?.text].filter(Boolean).join(' '))
  return [...hookNotices, ...assetNotices, ...new Set(otherNotices)]
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

export function register(on: On) {
  on('session.start', async ($, e, next) => {
    const result = await next(e)
    if (!e.isInteractive) return result
    const probe = await $.process.run(VERSION_ARGV, { cwd: e.cwd, timeoutMs: VERSION_TIMEOUT_MS }).catch(() => undefined)
    if (!probe) {
      missing = true
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
    const doctor = await $.process.run(DOCTOR_ARGV, { cwd: e.cwd }).catch(() => undefined)
    if (!doctor) return result
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
    return next(e)
  }).catch(($, e, next) => next(e))

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
