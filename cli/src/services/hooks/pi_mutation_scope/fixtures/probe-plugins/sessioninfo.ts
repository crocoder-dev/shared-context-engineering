import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { appendFileSync } from "node:fs";

const LOG = process.env.PI_PROBE_LOG as string;

export default function (pi: ExtensionAPI) {
  pi.on("session_start" as any, async (event: any, ctx: any) => {
    appendFileSync(
      LOG,
      JSON.stringify({
        hook: "session_info",
        sessionId: ctx.sessionManager?.getSessionId?.(),
        sessionFile: ctx.sessionManager?.getSessionFile?.(),
        model: ctx.model,
      }) + "\n",
    );
  });
}
