import fs from "node:fs"
const LOG = process.env.OC_PROBE_LOG || "/tmp/oc-probe.jsonl"
const TAG = process.env.OC_PROBE_TAG || "run"
function rec(o: Record<string, unknown>) {
  fs.appendFileSync(LOG, JSON.stringify({ tag: TAG, plugin: "order-last", wall: new Date().toISOString(), pid: process.pid, ...o }) + "\n")
}
export const orderLast = async () => {
  rec({ kind: "plugin.init" })
  return {
    "tool.execute.before": async (i: any) => {
      rec({ kind: "hook", hook: "tool.execute.before", plugin: "order-last", tool: i?.tool, callID: i?.callID })
    },
    "shell.env": async (i: any) => {
      rec({ kind: "hook", hook: "shell.env", plugin: "order-last", callID: i?.callID })
    },
    "tool.execute.after": async (i: any) => {
      rec({ kind: "hook", hook: "tool.execute.after", plugin: "order-last", tool: i?.tool, callID: i?.callID })
    },
  }
}
