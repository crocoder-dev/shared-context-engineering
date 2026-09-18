import {
	AuthStorage,
	createAgentSession,
	DefaultResourceLoader,
	ModelRegistry,
	SessionManager,
	SettingsManager,
} from "@earendil-works/pi-coding-agent";

const cwd = process.argv[2];
const agentDir = process.argv[3];
if (!cwd || !agentDir) {
	console.error("usage: driver.mjs <scratch-repo-cwd> <agent-dir>");
	process.exit(1);
}

async function main() {
	const authStorage = AuthStorage.create();
	const modelRegistry = ModelRegistry.create(authStorage);
	const settingsManager = SettingsManager.create(cwd, agentDir);
	const sessionManager = SessionManager.create(cwd, undefined);

	const resourceLoader = new DefaultResourceLoader({
		cwd,
		agentDir,
		settingsManager,
	});
	await resourceLoader.reload();

	const { session } = await createAgentSession({
		cwd,
		agentDir,
		thinkingLevel: "off",
		authStorage,
		modelRegistry,
		settingsManager,
		sessionManager,
		resourceLoader,
		noTools: false,
	});

	const model = modelRegistry.find("sce-test-provider", "sce-test-model");
	if (!model) {
		console.error(
			"FAIL: sce-test-provider/sce-test-model was not registered by extension load",
		);
		session.dispose();
		process.exit(2);
	}
	console.log(
		"Resolved custom model after session construction:",
		model.provider,
		model.id,
	);
	await session.setModel(model);

	const events = [];
	session.subscribe((event) => {
		events.push(event.type);
	});

	try {
		await session.prompt("please run the scripted bash tool call");
		await new Promise((r) => setTimeout(r, 300));
		await session.prompt("continue");
		await new Promise((r) => setTimeout(r, 300));
		console.log("DONE. event types seen:", JSON.stringify(events));
	} catch (err) {
		console.error("ERROR during session.prompt:", err);
		process.exitCode = 3;
	} finally {
		session.dispose();
	}
}

main();
