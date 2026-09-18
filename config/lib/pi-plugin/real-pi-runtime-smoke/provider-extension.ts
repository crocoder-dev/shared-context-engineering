import {
	createFauxCore,
	fauxAssistantMessage,
	fauxToolCall,
} from "@earendil-works/pi-ai";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export default function (pi: ExtensionAPI) {
	const core = createFauxCore({
		api: "sce-test-provider-api",
		provider: "sce-test-provider",
	});
	core.setResponses([
		fauxAssistantMessage(
			fauxToolCall(
				"bash",
				{ command: 'printf "sce-real-pi-smoke\\n" >> smoke-output.txt' },
				{ id: "sce-smoke-bash-1" },
			),
			{ stopReason: "toolUse" },
		),
		fauxAssistantMessage("done", { stopReason: "stop" }),
	]);

	pi.registerProvider("sce-test-provider", {
		name: "SCE Test Provider",
		baseUrl: "http://localhost:0/unused",
		apiKey: "unused-dummy-key",
		api: "sce-test-provider-api",
		models: [
			{
				id: "sce-test-model",
				name: "SCE Test Model",
				reasoning: false,
				input: ["text"],
				cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
				contextWindow: 128000,
				maxTokens: 4096,
			},
		],
		streamSimple: core.streamSimple,
	});
}
