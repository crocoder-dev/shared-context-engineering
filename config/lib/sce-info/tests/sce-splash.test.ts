import type { On } from 'claude-code'
import { expect, test } from 'claude-code/testing'
import { ART, FOOTER, LOADED, SITE } from '../hooks/register'

const BAND = {
  plugin: 'sce-splash',
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

// Stands in for what Claude Code (or a mod after this one) draws in the band
function drawBeneath(on: On) {
  on('ui.render', { component: 'AbovePrompt' }, ($, e) => {
    const { Text } = $.ui.resolve(e)
    return Text({ children: 'beneath' })
  })
}

test('the startup band shows the "SCE" art once and keeps what is beneath it', async ($, on) => {
  drawBeneath(on)
  const ui = await $.ui.mount(BAND)
  for (const row of ART) expect(await ui.findAll({ type: 'Text', text: row })).toHaveLength(1)
  expect(await ui.findAll({ type: 'Text', text: ` ${LOADED}` })).toHaveLength(1)
  expect(await ui.findAll({ type: 'Text', text: FOOTER })).toHaveLength(1)
  expect(await ui.findAll({ type: 'Link', text: SITE })).toHaveLength(1)
  expect(await ui.find({ type: 'Text', text: 'beneath' })).toBeDefined()

  // Redraws don't stack a second label
  await ui.redraw()
  await ui.redraw()
  for (const row of ART) expect(await ui.findAll({ type: 'Text', text: row })).toHaveLength(1)
})

test('the label yields to a survey', async ($, on) => {
  drawBeneath(on)
  const ui = await $.ui.mount({ ...BAND, props: { ...BAND.props, hasSurvey: true } })
  expect(await ui.find({ type: 'Text', text: ART[0] })).toBeUndefined()
  expect(await ui.find({ type: 'Text', text: 'beneath' })).toBeDefined()
})

test('the label leaves once the first prompt is sent', async ($, on) => {
  drawBeneath(on)
  // Answer in Claude Code's place, so the prompt goes nowhere
  on('prompt.submit', ($, e) => ({ text: e.text }))
  const ui = await $.ui.mount(BAND)
  expect(await ui.find({ type: 'Text', text: ART[0] })).toBeDefined()

  await $.prompt.submit({ text: 'hello', wait: false, origin: { kind: 'composer' } })
  expect(await ui.find({ type: 'Text', text: ART[0] })).toBeUndefined()
  expect(await ui.find({ type: 'Text', text: 'beneath' })).toBeDefined()
})

test('the label comes back after a /clear', async ($, on) => {
  drawBeneath(on)
  on('prompt.submit', ($, e) => ({ text: e.text }))
  // Claude Code's own end step
  on('session.end', ($, e) => ({ sessionId: e.sessionId }))
  const ui = await $.ui.mount(BAND)
  await $.prompt.submit({ text: 'hello', wait: false, origin: { kind: 'composer' } })
  expect(await ui.find({ type: 'Text', text: ART[0] })).toBeUndefined()

  await $.session.end({ reason: 'clear', sessionId: 's1', resume: { id: 's1' } })
  expect(await ui.findAll({ type: 'Text', text: ART[0] })).toHaveLength(1)

  // And leaves again on the next prompt
  await $.prompt.submit({ text: 'again', wait: false, origin: { kind: 'composer' } })
  expect(await ui.find({ type: 'Text', text: ART[0] })).toBeUndefined()
})
