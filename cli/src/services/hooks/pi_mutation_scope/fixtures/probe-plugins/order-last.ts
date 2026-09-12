import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { appendFileSync } from "node:fs";

const LOG = process.env.PI_PROBE_LOG as string;
let seq = 0;

function write(hook: string, payload: unknown) {
  seq += 1;
  appendFileSync(
    LOG,
    JSON.stringify({
      mono_us: Number(process.hrtime.bigint() / 1000n),
      pid: process.pid,
      tag: "order-last",
      seq,
      hook,
      payload,
    }) + "\n",
  );
}

export default function (pi: ExtensionAPI) {
  pi.on("tool_call" as any, async (event: any) => {
    write("tool_call", { toolName: event.toolName, toolCallId: event.toolCallId });
  });
  pi.on("tool_execution_start" as any, async (event: any) => {
    write("tool_execution_start", { toolName: event.toolName, toolCallId: event.toolCallId });
  });
}
