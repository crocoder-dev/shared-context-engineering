#!/usr/bin/env node

import { spawn } from "node:child_process";
import { accessSync, constants as fsConstants } from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

import {
	formatUnsupportedPlatformMessage,
	getInstalledBinaryPath,
} from "../lib/platform.js";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

function ensureInstalledBinaryPath() {
	const binaryPath = getInstalledBinaryPath(__dirname);

	try {
		accessSync(binaryPath, fsConstants.X_OK);
		return binaryPath;
	} catch {
		return null;
	}
}

const binaryPath = ensureInstalledBinaryPath();

if (!binaryPath) {
	const unsupportedMessage = formatUnsupportedPlatformMessage(
		process.platform,
		process.arch,
	);

	if (unsupportedMessage) {
		console.error(unsupportedMessage);
	} else {
		console.error(
			"The native sce binary is not installed. Try reinstalling the package or run `npm rebuild sce` to retry the download.",
		);
	}

	process.exit(1);
}

const child = spawn(binaryPath, process.argv.slice(2), {
	stdio: "inherit",
	env: process.env,
});

child.on("exit", (code, signal) => {
	if (signal) {
		process.kill(process.pid, signal);
		return;
	}

	process.exit(code ?? 1);
});

child.on("error", (error) => {
	console.error(`Failed to launch sce: ${error.message}`);
	process.exit(1);
});
