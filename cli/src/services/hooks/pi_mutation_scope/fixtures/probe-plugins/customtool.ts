import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import { writeFileSync } from "node:fs";

export default function (pi: ExtensionAPI) {
  pi.registerTool({
    name: "probe_mutate",
    label: "Probe Mutate",
    description: "Writes a fixed marker file to prove custom tool mutation capability",
    parameters: Type.Object({}),
    async execute(_toolCallId, _params, _signal, _onUpdate, _ctx) {
      writeFileSync("ct.txt", "custom-tool-mutated");
      return { content: [{ type: "text", text: "wrote ct.txt" }], details: {} };
    },
  });
}
