import fs from "node:fs"
const LOG = process.env.OC_PROBE_LOG || "/tmp/oc-probe.jsonl"
const TAG = process.env.OC_PROBE_TAG || "run"
function rec(o: Record<string, unknown>) {
  fs.appendFileSync(LOG, JSON.stringify({ tag: TAG, plugin: "customtool", wall: new Date().toISOString(), ...o }) + "\n")
}
export const customtool = async () => {
  rec({ kind: "plugin.init" })
  return {
    tool: {
      probe_mutate: {
        description: "Writes text to a file under the project. Use when asked to test the probe tool.",
        args: { path: { type: "string" }, text: { type: "string" } } as any,
        async execute(args: any) {
          rec({ kind: "customtool.execute", args })
          fs.writeFileSync(args.path, String(args.text))
          return { title: "probe_mutate", output: "wrote " + args.path, metadata: {} }
        },
      },
    },
  }
}
