import { spawnSync } from "node:child_process";
import { createHash, createVerify } from "node:crypto";
import {
	chmodSync,
	copyFileSync,
	createReadStream,
	createWriteStream,
	existsSync,
	mkdirSync,
	mkdtempSync,
	readFileSync,
	realpathSync,
	rmSync,
} from "node:fs";
import { get } from "node:https";
import { tmpdir } from "node:os";
import path from "node:path";
import process from "node:process";
import { pipeline } from "node:stream/promises";
import { fileURLToPath } from "node:url";

import {
	formatUnsupportedPlatformMessage,
	getArchiveName,
	getArchiveRoot,
	getInstalledBinaryPath,
	getReleaseManifestName,
	resolveSupportedPlatform,
	selectReleaseArtifact,
} from "./platform.js";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const PACKAGE_ROOT = path.resolve(__dirname, "..");
const PACKAGE_JSON_PATH = path.join(PACKAGE_ROOT, "package.json");
const MANIFEST_PUBLIC_KEY_PATH = path.join(
	__dirname,
	"release-manifest-public-key.pem",
);
const RUNTIME_DIR = path.join(PACKAGE_ROOT, "runtime");
const DOWNLOAD_TIMEOUT_MS = 30_000;
const DEFAULT_REPOSITORY_SLUG = "crocoder-dev/shared-context-engineering";

function coerceManifestPayload(manifestPayload) {
	if (typeof manifestPayload === "string" || Buffer.isBuffer(manifestPayload)) {
		return manifestPayload;
	}

	return JSON.stringify(manifestPayload);
}

function decodeManifestSignature(signaturePayload) {
	if (Buffer.isBuffer(signaturePayload)) {
		return signaturePayload;
	}

	if (
		typeof signaturePayload !== "string" ||
		signaturePayload.trim().length === 0
	) {
		throw new Error(
			"Release manifest signature payload must be a non-empty base64 string or Buffer.",
		);
	}

	return Buffer.from(signaturePayload.trim(), "base64");
}

export function readBundledReleaseManifestPublicKey(
	publicKeyPath = MANIFEST_PUBLIC_KEY_PATH,
) {
	return readFileSync(publicKeyPath, "utf8");
}

export function verifyReleaseManifestSignature(
	manifestPayload,
	signaturePayload,
	publicKeyPem = readBundledReleaseManifestPublicKey(),
) {
	const verifier = createVerify("sha256");
	verifier.update(coerceManifestPayload(manifestPayload));
	verifier.end();

	return verifier.verify(
		publicKeyPem,
		decodeManifestSignature(signaturePayload),
	);
}

function readPackageVersion() {
	const packageJson = JSON.parse(readFileSync(PACKAGE_JSON_PATH, "utf8"));

	if (!packageJson.version || typeof packageJson.version !== "string") {
		throw new Error("Invalid npm package metadata: missing version.");
	}

	return packageJson.version;
}

function getRepositorySlug() {
	return process.env.GITHUB_REPOSITORY ?? DEFAULT_REPOSITORY_SLUG;
}

export function getReleaseBaseUrl(version) {
	return (
		process.env.SCE_NPM_RELEASE_BASE_URL ??
		`https://github.com/${getRepositorySlug()}/releases/download/v${version}`
	);
}

function getManifestUrl(version) {
	return `${getReleaseBaseUrl(version)}/${getReleaseManifestName(version)}`;
}

function getManifestSignatureUrl(version) {
	return `${getManifestUrl(version)}.sig`;
}

async function downloadToFile(url, destinationPath, redirectsRemaining = 5) {
	await new Promise((resolve, reject) => {
		let settled = false;
		let timeoutId;

		const finish = (handler, value) => {
			if (settled) {
				return;
			}

			settled = true;
			clearTimeout(timeoutId);
			request.removeListener("error", handleError);
			handler(value);
		};

		const handleError = (error) => {
			finish(reject, error);
		};

		const request = get(url, (response) => {
			const { statusCode = 0, headers } = response;

			if (statusCode >= 300 && statusCode < 400 && headers.location) {
				clearTimeout(timeoutId);
				response.resume();

				if (redirectsRemaining <= 0) {
					finish(
						reject,
						new Error(`Too many redirects while downloading ${url}.`),
					);
					return;
				}

				const redirectedUrl = new URL(headers.location, url).toString();
				downloadToFile(redirectedUrl, destinationPath, redirectsRemaining - 1)
					.then((value) => finish(resolve, value))
					.catch((error) => finish(reject, error));
				return;
			}

			if (statusCode !== 200) {
				clearTimeout(timeoutId);
				response.resume();
				finish(
					reject,
					new Error(
						`Unexpected response ${statusCode} while downloading ${url}.`,
					),
				);
				return;
			}

			const output = createWriteStream(destinationPath);
			pipeline(response, output)
				.then((value) => finish(resolve, value))
				.catch((error) => finish(reject, error));
		});

		timeoutId = setTimeout(() => {
			const error = new Error(
				`Request timed out after ${DOWNLOAD_TIMEOUT_MS}ms while downloading ${url}.`,
			);
			request.destroy(error);
		}, DOWNLOAD_TIMEOUT_MS);

		request.on("error", handleError);
	});
}

async function downloadText(url) {
	const tempDir = mkdtempSync(path.join(tmpdir(), "sce-npm-text-"));
	const textPath = path.join(tempDir, "payload.txt");

	try {
		await downloadToFile(url, textPath);
		return readFileSync(textPath, "utf8");
	} finally {
		rmSync(tempDir, { recursive: true, force: true });
	}
}

export function parseVerifiedReleaseManifest(
	manifestPayload,
	signaturePayload,
	publicKeyPem = readBundledReleaseManifestPublicKey(),
) {
	let manifest;

	try {
		manifest = JSON.parse(coerceManifestPayload(manifestPayload).toString());
	} catch {
		throw new Error(
			"Invalid sce release manifest: failed to parse JSON payload.",
		);
	}

	try {
		if (
			!verifyReleaseManifestSignature(
				manifestPayload,
				signaturePayload,
				publicKeyPem,
			)
		) {
			throw new Error();
		}
	} catch {
		throw new Error(
			"Release manifest authenticity check failed: signature verification did not succeed.",
		);
	}

	return manifest;
}

export async function loadVerifiedReleaseManifest(
	version,
	{
		downloadManifest = downloadText,
		downloadManifestSignature = downloadText,
		publicKeyPem = readBundledReleaseManifestPublicKey(),
	} = {},
) {
	let manifestPayload;

	try {
		manifestPayload = await downloadManifest(getManifestUrl(version));
	} catch (error) {
		throw new Error(
			`Failed to download sce release manifest: ${error.message}`,
		);
	}

	let signaturePayload;

	try {
		signaturePayload = await downloadManifestSignature(
			getManifestSignatureUrl(version),
		);
	} catch (error) {
		throw new Error(
			`Failed to download sce release manifest signature: ${error.message}`,
		);
	}

	return parseVerifiedReleaseManifest(
		manifestPayload,
		signaturePayload,
		publicKeyPem,
	);
}

function sha256File(filePath) {
	return new Promise((resolve, reject) => {
		const hash = createHash("sha256");
		const input = createReadStream(filePath);

		input.on("data", (chunk) => {
			hash.update(chunk);
		});

		input.on("end", () => {
			resolve(hash.digest("hex"));
		});

		input.on("error", (error) => {
			reject(error);
		});
	});
}

function extractArchive(archivePath, destinationDir) {
	const tarResult = spawnSync(
		"tar",
		["-xzf", archivePath, "-C", destinationDir],
		{
			stdio: "pipe",
			encoding: "utf8",
		},
	);

	if (tarResult.error) {
		throw new Error(
			`Failed to extract sce release archive: ${tarResult.error.message}`,
		);
	}

	if (tarResult.status !== 0) {
		throw new Error(
			tarResult.stderr?.trim() ||
				tarResult.error?.message ||
				"Failed to extract sce release archive.",
		);
	}
}

export async function installBinaryWithDependencies({
	supportedPlatform = resolveSupportedPlatform(),
	unsupportedMessage = formatUnsupportedPlatformMessage(),
	version = readPackageVersion(),
	loadReleaseManifest = loadVerifiedReleaseManifest,
	downloadArchive = downloadToFile,
	checksumFile = sha256File,
	extractArchiveFn = extractArchive,
	fileExists = existsSync,
	createRuntimeDir = mkdirSync,
	copyBinary = copyFileSync,
	chmodBinary = chmodSync,
	createTempDir = () => mkdtempSync(path.join(tmpdir(), "sce-npm-install-")),
	removeTempDir = (tempDir) =>
		rmSync(tempDir, { recursive: true, force: true }),
} = {}) {
	if (process.env.SCE_NPM_SKIP_DOWNLOAD === "1") {
		console.log(
			"Skipping sce binary download because SCE_NPM_SKIP_DOWNLOAD=1.",
		);
		return;
	}

	if (!supportedPlatform) {
		throw new Error(
			unsupportedMessage ?? "Unsupported platform for npm sce package.",
		);
	}

	const releaseManifest = await loadReleaseManifest(version);
	const artifact = selectReleaseArtifact(
		releaseManifest,
		supportedPlatform.targetTriple,
	);
	const archiveName =
		artifact.archive ?? getArchiveName(version, supportedPlatform.targetTriple);
	const expectedChecksum = artifact.checksum_sha256;

	if (!expectedChecksum) {
		throw new Error(
			`Release artifact ${archiveName} is missing checksum_sha256 metadata.`,
		);
	}

	const tempDir = createTempDir();

	try {
		const archivePath = path.join(tempDir, archiveName);
		const archiveUrl = `${getReleaseBaseUrl(version)}/${archiveName}`;

		await downloadArchive(archiveUrl, archivePath);

		const actualChecksum = await checksumFile(archivePath);
		if (actualChecksum !== expectedChecksum) {
			throw new Error(
				`Downloaded sce archive checksum mismatch for ${archiveName}: expected ${expectedChecksum}, received ${actualChecksum}.`,
			);
		}

		extractArchiveFn(archivePath, tempDir);

		const extractedBinaryPath = path.join(
			tempDir,
			getArchiveRoot(version, supportedPlatform.targetTriple),
			"bin",
			"sce",
		);

		if (!fileExists(extractedBinaryPath)) {
			throw new Error(
				`Extracted sce archive did not contain ${archiveName} -> bin/sce.`,
			);
		}

		createRuntimeDir(RUNTIME_DIR, { recursive: true });
		const installedBinaryPath = getInstalledBinaryPath(__dirname);
		copyBinary(extractedBinaryPath, installedBinaryPath);
		chmodBinary(installedBinaryPath, 0o755);
	} finally {
		removeTempDir(tempDir);
	}
}

export async function installBinary() {
	await installBinaryWithDependencies();
}

if (
	process.argv[1] &&
	realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))
) {
	installBinary().catch((error) => {
		console.error(`Failed to install sce via npm: ${error.message}`);
		process.exit(1);
	});
}
