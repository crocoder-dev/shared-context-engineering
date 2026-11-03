import {
	createReadStream,
	readFileSync,
	statSync,
	writeFileSync,
} from "node:fs";
import { createServer } from "node:https";
import path from "node:path";
import process from "node:process";

const LOOPBACK_HOST = "127.0.0.1";

function readFlagValue(argv, index, flagName) {
	const value = argv[index + 1];

	if (!value || value.startsWith("-")) {
		throw new Error(`Missing value for ${flagName}.`);
	}

	return value;
}

function parseArgs(argv) {
	const options = { rootDir: "", certFile: "", keyFile: "", portFile: "" };
	const flags = new Map([
		["--root", "rootDir"],
		["--cert", "certFile"],
		["--key", "keyFile"],
		["--port-file", "portFile"],
	]);

	for (let index = 0; index < argv.length; index += 2) {
		const optionName = flags.get(argv[index]);

		if (!optionName) {
			throw new Error(`Unknown argument: ${argv[index]}`);
		}

		options[optionName] = readFlagValue(argv, index, argv[index]);
	}

	for (const [flagName, optionName] of flags) {
		if (!options[optionName]) {
			throw new Error(`${flagName} is required.`);
		}
	}

	return options;
}

function resolveRequestPath(rootDir, requestUrl) {
	const { pathname } = new URL(requestUrl, `https://${LOOPBACK_HOST}`);
	const fileName = decodeURIComponent(pathname).replace(/^\/+/, "");

	if (fileName.length === 0 || fileName.includes("/")) {
		return null;
	}

	const filePath = path.join(rootDir, fileName);

	try {
		return statSync(filePath).isFile() ? filePath : null;
	} catch {
		return null;
	}
}

const { rootDir, certFile, keyFile, portFile } = parseArgs(
	process.argv.slice(2),
);

const server = createServer(
	{ cert: readFileSync(certFile), key: readFileSync(keyFile) },
	(request, response) => {
		const filePath =
			request.method === "GET"
				? resolveRequestPath(rootDir, request.url ?? "/")
				: null;

		if (!filePath) {
			response.writeHead(404);
			response.end();
			return;
		}

		response.writeHead(200, { "content-type": "application/octet-stream" });
		createReadStream(filePath).pipe(response);
	},
);

server.listen(0, LOOPBACK_HOST, () => {
	writeFileSync(portFile, `${server.address().port}\n`, "utf8");
});

process.on("SIGTERM", () => {
	server.close(() => process.exit(0));
});
