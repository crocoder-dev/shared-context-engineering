import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { appendFileSync } from "node:fs";

const LOG = process.env.PI_PROBE_LOG as string;
const TAG = process.env.PI_PROBE_TAG || "capture";
let seq = 0;

function write(hook: string, payload: unknown) {
  seq += 1;
  const line = {
    mono_us: Number(process.hrtime.bigint() / 1000n),
    wall: new Date().toISOString(),
    pid: process.pid,
    tag: TAG,
    seq,
    hook,
    payload,
  };
  appendFileSync(LOG, JSON.stringify(line) + "\n");
}

export default function (pi: ExtensionAPI) {
  const events = [
    "session_start",
    "session_shutdown",
    "before_agent_start",
    "agent_start",
    "agent_end",
    "agent_settled",
    "turn_start",
    "turn_end",
    "tool_call",
    "tool_execution_start",
    "tool_execution_end",
    "tool_result",
    "user_bash",
    "model_select",
  ] as const;

  for (const name of events) {
    pi.on(name as any, async (event: any, ctx: any) => {
      const model = ctx?.model ? { provider: ctx.model.provider, id: ctx.model.id } : undefined;
      write(name, { event, model });

      if (name === "tool_call") {
        const fault = process.env.PI_PROBE_FAULT;
        if (fault === "throw" && TAG === (process.env.PI_PROBE_FAULT_TAG || TAG)) {
          write("tool_call_fault_throw", { toolName: event.toolName, toolCallId: event.toolCallId });
          throw new Error(`PI_PROBE_FAULT throw (${TAG})`);
        }
        if (fault === "block" && TAG === (process.env.PI_PROBE_FAULT_TAG || TAG)) {
          write("tool_call_fault_block", { toolName: event.toolName, toolCallId: event.toolCallId });
          return { block: true, reason: `PI_PROBE_FAULT block (${TAG})` };
        }
      }
    });
  }
}
