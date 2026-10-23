import type { On } from 'claude-code'

let started = false

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
    GRADIENT_LEFT.map((a, i) => Math.round(a + (GRADIENT_RIGHT[i] - a) * t).toString(16).padStart(2, '0')).join('')
  )
}

export const SITE = 'sce.crocoder.dev'
export const FOOTER = `Visit ${SITE} to learn more.`
export const LOADED = 'loaded.'
export const LOGIN_REQUIRED = "Login required for this repository. Run 'sce auth login'."
export const DOCTOR_ARGV = ['sce', 'doctor', '--format', 'json'] as const

export function loginRequired(doctorJson: string): boolean {
  try {
    return JSON.parse(doctorJson)?.sce_login?.state === 'not_logged_in'
  } catch {
    return false
  }
}

export function register(on: On) {
  on('session.start', async ($, e, next) => {
    const result = await next(e)
    if (!e.isInteractive) return result
    const doctor = await $.process.run(DOCTOR_ARGV, { cwd: e.cwd }).catch(() => undefined)
    if (doctor && loginRequired(doctor.stdout)) $.ui.log(LOGIN_REQUIRED)
    return result
  })

  on('prompt.submit', async ($, e, next) => {
    if (!started) {
      started = true
      $.ui.invalidate('ui.render')
    }
    return next(e)
  }).catch(($, e, next) => next(e))

  on('session.end', async ($, e, next) => {
    if (e.reason === 'clear' && started) {
      started = false
      $.ui.invalidate('ui.render')
    }
    return next(e)
  })

  on('ui.render', { component: 'AbovePrompt', surface: 'terminal' }, async ($, e, next) => {
    if (started || e.props.hasSurvey) return next(e)
    const { Box, Text, Link } = $.ui.resolve(e)
    const theirs = await next(e)
    const label = Box({
      paddingLeft: 11,
      flexDirection: 'column',
      children: [
        ...ART.map((row, i) => {
          const art = Text({ children: [...row].map((ch, col) => (ch === ' ' ? ch : Text({ color: gradientColor(col), children: ch }))) })
          if (i !== ART.length - 2) return art
          return Box({ flexDirection: 'row', children: [art, Text({ color: 'cyan', children: ` ${LOADED}` })] })
        }),
        Text({
          children: [
            'Visit ',
            Text({ color: 'cyan', underline: true, children: Link({ href: `https://${SITE}`, label: SITE }) }),
            ' to learn more.',
          ],
        }),
      ],
    })
    return Box({ flexDirection: 'column', children: [label, theirs] })
  })
}
