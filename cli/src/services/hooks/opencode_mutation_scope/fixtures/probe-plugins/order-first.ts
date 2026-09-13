import fs from "node:fs"
const LOG = process.env.OC_PROBE_LOG || "/tmp/oc-probe.jsonl"
const TAG = process.env.OC_PROBE_TAG || "run"
const MODE = process.env.OC_PROBE_ORDER_FIRST || "observe"
function rec(o: Record<string, unknown>) {
  fs.appendFileSync(LOG, JSON.stringify({ tag: TAG, plugin: "order-first", wall: new Date().toISOString(), pid: process.pid, ...o }) + "\n")
}
export const orderFirst = async () => {
  rec({ kind: "plugin.init" })
  return {
    "tool.execute.before": async (i: any) => {
      rec({ kind: "hook", hook: "tool.execute.before", plugin: "order-first", tool: i?.tool, callID: i?.callID })
      if (MODE === "throw") {
        rec({ kind: "fault", where: "tool.execute.before", plugin: "order-first" })
        throw new Error("order-first synchronous throw")
      }
    },
    "shell.env": async (i: any) => {
      rec({ kind: "hook", hook: "shell.env", plugin: "order-first", callID: i?.callID })
    },
    "tool.execute.after": async (i: any) => {
      rec({ kind: "hook", hook: "tool.execute.after", plugin: "order-first", tool: i?.tool, callID: i?.callID })
    },
  }
}
