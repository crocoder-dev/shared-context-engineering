import fs from "node:fs"

const LOG = process.env.OC_PROBE_LOG || "/tmp/oc-probe.jsonl"
const TAG = process.env.OC_PROBE_TAG || "run"
const NAME = process.env.OC_PROBE_NAME || "capture"
const FAULT = process.env.OC_PROBE_FAULT || ""

let seq = 0
const start = process.hrtime.bigint()

function rec(obj: Record<string, unknown>) {
  seq += 1
  const line = JSON.stringify({
    seq,
    tag: TAG,
    plugin: NAME,
    mono_us: Number((process.hrtime.bigint() - start) / 1000n),
    wall: new Date().toISOString(),
    pid: process.pid,
    ...obj,
  })
  fs.appendFileSync(LOG, line + "\n")
}

function safe(v: unknown) {
  try {
    return JSON.parse(JSON.stringify(v, (_k, val) => (typeof val === "bigint" ? String(val) : val)))
  } catch {
    return String(v)
  }
}

export const capture = async (input: any) => {
  rec({ kind: "plugin.init", directory: input?.directory, worktree: input?.worktree })

  return {
    event: async ({ event }: any) => {
      rec({ kind: "event", type: event?.type, properties: safe(event?.properties) })
    },
    config: async (cfg: any) => {
      rec({ kind: "hook", hook: "config", pluginList: safe(cfg?.plugin) })
    },
    "chat.message": async (i: any, o: any) => {
      rec({ kind: "hook", hook: "chat.message", sessionID: i?.sessionID, agent: i?.agent, model: safe(i?.model) })
    },
    "chat.params": async (i: any, _o: any) => {
      rec({
        kind: "hook",
        hook: "chat.params",
        sessionID: i?.sessionID,
        agent: i?.agent,
        model_id: i?.model?.id,
        model_providerID: i?.model?.providerID,
        model_api_id: i?.model?.api?.id,
        provider_source: i?.provider?.source,
        message_id: i?.message?.id,
      })
    },
    "permission.ask": async (i: any, o: any) => {
      rec({ kind: "hook", hook: "permission.ask", input: safe(i), status_out: o?.status })
    },
    "tool.execute.before": async (i: any, o: any) => {
      rec({ kind: "hook", hook: "tool.execute.before", input: safe(i), args: safe(o?.args) })
      if (FAULT === "before") {
        rec({ kind: "fault", where: "tool.execute.before", plugin: NAME })
        throw new Error(`OC_PROBE_FAULT before (${NAME})`)
      }
    },
    "shell.env": async (i: any, o: any) => {
      rec({ kind: "hook", hook: "shell.env", input: safe(i), env_keys_out: Object.keys(o?.env || {}) })
      if (FAULT === "shellenv") {
        rec({ kind: "fault", where: "shell.env", plugin: NAME })
        throw new Error(`OC_PROBE_FAULT shellenv (${NAME})`)
      }
    },
    "tool.execute.after": async (i: any, o: any) => {
      rec({
        kind: "hook",
        hook: "tool.execute.after",
        input: safe(i),
        title: o?.title,
        output_preview: typeof o?.output === "string" ? o.output.slice(0, 300) : safe(o?.output),
        metadata: safe(o?.metadata),
      })
    },
  }
}
