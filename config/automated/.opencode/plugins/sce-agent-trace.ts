import type { Plugin } from "@opencode-ai/plugin";

export const SceAgentTracePlugin: Plugin = async () => {
 
  return {
    "file.edited": async (input, output) => {
      console.log("input/output", input, output)
    },
  };
};
