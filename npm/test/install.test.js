import { afterEach, describe, expect, test } from "bun:test";
import { createHash, generateKeyPairSync, sign } from "node:crypto";
import { existsSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import {
	createReleaseManifestSignature,
	resolveReleaseManifestSigningKey,
	SIGNING_KEY_ENV_VAR,
} from "../../scripts/lib/release-manifest-signing.mjs";
import {
	getReleaseBaseUrl,
	installBinaryWithDependencies,
	loadVerifiedReleaseManifest,
	parseVerifiedReleaseManifest,
	readBundledReleaseManifestPublicKey,
	verifyReleaseManifestSignature,
} from "../lib/install.js";
import { getInstalledBinaryPath } from "../lib/platform.js";

function createManifestFixture(overrides = {}) {
	const manifest = {
		version: "0.1.0",
		artifacts: [
			{ target_triple: "x86_64-unknown-linux-gnu", checksum_sha256: "abc123" },
		],
		...overrides,
	};
	const manifestPayload = JSON.stringify(manifest);
	const { publicKey, privateKey } = generateKeyPairSync("rsa", {
		modulusLength: 2048,
		publicKeyEncoding: { type: "spki", format: "pem" },
		privateKeyEncoding: { type: "pkcs8", format: "pem" },
	});
	const signature = sign(
		"sha256",
		Buffer.from(manifestPayload),
		privateKey,
	).toString("base64");

	return { manifest, manifestPayload, publicKey, signature };
}

const tempDirs = [];

afterEach(() => {
	for (const tempDir of tempDirs.splice(0)) {
		rmSync(tempDir, { recursive: true, force: true });
	}
});

describe("release manifest verification primitives", () => {
	test("builds the default release base URL from the publisher repository slug", () => {
		const originalRepository = process.env.GITHUB_REPOSITORY;
		const originalReleaseBaseUrl = process.env.SCE_NPM_RELEASE_BASE_URL;

		delete process.env.SCE_NPM_RELEASE_BASE_URL;
		process.env.GITHUB_REPOSITORY = "someone-else/caller-repo";

		try {
			expect(getReleaseBaseUrl("0.1.0")).toBe(
				"https://github.com/crocoder-dev/shared-context-engineering/releases/download/v0.1.0",
			);
		} finally {
			if (originalRepository === undefined) {
				delete process.env.GITHUB_REPOSITORY;
			} else {
				process.env.GITHUB_REPOSITORY = originalRepository;
			}

			if (originalReleaseBaseUrl === undefined) {
				delete process.env.SCE_NPM_RELEASE_BASE_URL;
			} else {
				process.env.SCE_NPM_RELEASE_BASE_URL = originalReleaseBaseUrl;
			}
		}
	});

	test("prefers an explicit release base URL override", () => {
		const originalReleaseBaseUrl = process.env.SCE_NPM_RELEASE_BASE_URL;

		process.env.SCE_NPM_RELEASE_BASE_URL = "https://example.com/releases";

		try {
			expect(getReleaseBaseUrl("0.1.0")).toBe("https://example.com/releases");
		} finally {
			if (originalReleaseBaseUrl === undefined) {
				delete process.env.SCE_NPM_RELEASE_BASE_URL;
			} else {
				process.env.SCE_NPM_RELEASE_BASE_URL = originalReleaseBaseUrl;
			}
		}
	});

	test("loads the bundled release manifest public key", () => {
		const publicKeyPem = readBundledReleaseManifestPublicKey();

		expect(publicKeyPem).toContain("BEGIN PUBLIC KEY");
	});

	test("accepts a valid detached manifest signature", () => {
		const { manifestPayload, publicKey, signature } = createManifestFixture();

		expect(
			verifyReleaseManifestSignature(manifestPayload, signature, publicKey),
		).toBe(true);
	});

	test("rejects a tampered manifest payload", () => {
		const { publicKey, privateKey } = generateKeyPairSync("rsa", {
			modulusLength: 2048,
			publicKeyEncoding: { type: "spki", format: "pem" },
			privateKeyEncoding: { type: "pkcs8", format: "pem" },
		});
		const manifestPayload = JSON.stringify({ version: "0.1.0", artifacts: [] });
		const tamperedPayload = JSON.stringify({ version: "0.1.1", artifacts: [] });
		const signature = sign(
			"sha256",
			Buffer.from(manifestPayload),
			privateKey,
		).toString("base64");

		expect(
			verifyReleaseManifestSignature(tamperedPayload, signature, publicKey),
		).toBe(false);
	});

	test("rejects a tampered signature payload", () => {
		const { publicKey, privateKey } = generateKeyPairSync("rsa", {
			modulusLength: 2048,
			publicKeyEncoding: { type: "spki", format: "pem" },
			privateKeyEncoding: { type: "pkcs8", format: "pem" },
		});
		const manifestPayload = JSON.stringify({ version: "0.1.0", artifacts: [] });
		const signatureBuffer = sign(
			"sha256",
			Buffer.from(manifestPayload),
			privateKey,
		);
		signatureBuffer[0] ^= 0xff;

		expect(
			verifyReleaseManifestSignature(
				manifestPayload,
				signatureBuffer.toString("base64"),
				publicKey,
			),
		).toBe(false);
	});

	test("accepts signatures produced by the release signing helper", () => {
		const manifestPayload = JSON.stringify({ version: "0.1.0", artifacts: [] });
		const { publicKey, privateKey } = generateKeyPairSync("rsa", {
			modulusLength: 2048,
			publicKeyEncoding: { type: "spki", format: "pem" },
			privateKeyEncoding: { type: "pkcs8", format: "pem" },
		});
		const signature = createReleaseManifestSignature(
			Buffer.from(manifestPayload),
			privateKey,
		);

		expect(
			verifyReleaseManifestSignature(manifestPayload, signature, publicKey),
		).toBe(true);
	});

	test("rejects non-byte manifest payloads when signing", () => {
		const { privateKey } = generateKeyPairSync("rsa", {
			modulusLength: 2048,
			publicKeyEncoding: { type: "spki", format: "pem" },
			privateKeyEncoding: { type: "pkcs8", format: "pem" },
		});

		expect(() =>
			createReleaseManifestSignature(
				{ version: "0.1.0", artifacts: [] },
				privateKey,
			),
		).toThrow(new TypeError("manifestPayload must be a Buffer or Uint8Array."));
	});

	test("loads the release signing key from the configured environment variable", () => {
		const privateKeyPem =
			"-----BEGIN PRIVATE KEY-----\nexample\n-----END PRIVATE KEY-----\n";

		expect(
			resolveReleaseManifestSigningKey({
				env: { [SIGNING_KEY_ENV_VAR]: privateKeyPem },
			}),
		).toBe(privateKeyPem);
	});
});

describe("verified release manifest loading", () => {
	test("parses and returns a valid signed manifest payload", async () => {
		const { manifest, manifestPayload, publicKey, signature } =
			createManifestFixture();

		await expect(
			loadVerifiedReleaseManifest("0.1.0", {
				downloadManifest: async () => manifestPayload,
				downloadManifestSignature: async () => signature,
				publicKeyPem: publicKey,
			}),
		).resolves.toEqual(manifest);
	});

	test("rejects an invalid signed manifest payload", () => {
		const { manifestPayload, publicKey } = createManifestFixture();

		expect(() =>
			parseVerifiedReleaseManifest(
				manifestPayload,
				"not-a-valid-signature",
				publicKey,
			),
		).toThrow(
			"Release manifest authenticity check failed: signature verification did not succeed.",
		);
	});

	test("surfaces a missing manifest signature download as an authenticity failure prerequisite", async () => {
		const { manifestPayload, publicKey } = createManifestFixture();

		await expect(
			loadVerifiedReleaseManifest("0.1.0", {
				downloadManifest: async () => manifestPayload,
				downloadManifestSignature: async () => {
					throw new Error(
						"Unexpected response 404 while downloading signature.",
					);
				},
				publicKeyPem: publicKey,
			}),
		).rejects.toThrow(
			"Failed to download sce release manifest signature: Unexpected response 404 while downloading signature.",
		);
	});
});

describe("npm installer trust flow", () => {
	test("aborts before archive download when manifest signature verification fails", async () => {
		const { manifestPayload, publicKey } = createManifestFixture();
		let archiveDownloads = 0;

		await expect(
			installBinaryWithDependencies({
				supportedPlatform: {
					targetTriple: "x86_64-unknown-linux-gnu",
					os: "linux",
					arch: "x64",
				},
				version: "0.1.0",
				loadReleaseManifest: () =>
					loadVerifiedReleaseManifest("0.1.0", {
						downloadManifest: async () => manifestPayload,
						downloadManifestSignature: async () => "broken-signature",
						publicKeyPem: publicKey,
					}),
				downloadArchive: async () => {
					archiveDownloads += 1;
				},
			}),
		).rejects.toThrow(
			"Release manifest authenticity check failed: signature verification did not succeed.",
		);

		expect(archiveDownloads).toBe(0);
	});

	test("aborts before archive download when manifest signature is missing", async () => {
		const { manifestPayload, publicKey } = createManifestFixture();
		let archiveDownloads = 0;

		await expect(
			installBinaryWithDependencies({
				supportedPlatform: {
					targetTriple: "x86_64-unknown-linux-gnu",
					os: "linux",
					arch: "x64",
				},
				version: "0.1.0",
				loadReleaseManifest: () =>
					loadVerifiedReleaseManifest("0.1.0", {
						downloadManifest: async () => manifestPayload,
						downloadManifestSignature: async () => {
							throw new Error(
								"Unexpected response 404 while downloading signature.",
							);
						},
						publicKeyPem: publicKey,
					}),
				downloadArchive: async () => {
					archiveDownloads += 1;
				},
			}),
		).rejects.toThrow(
			"Failed to download sce release manifest signature: Unexpected response 404 while downloading signature.",
		);

		expect(archiveDownloads).toBe(0);
	});

	test("fails archive checksum verification only after manifest verification succeeds", async () => {
		const archiveContents = "signed-manifest-verified-archive";
		const expectedChecksum = createHash("sha256")
			.update(archiveContents)
			.digest("hex");
		const { manifestPayload, publicKey, signature } = createManifestFixture({
			artifacts: [
				{
					target_triple: "x86_64-unknown-linux-gnu",
					checksum_sha256: expectedChecksum,
				},
			],
		});
		let extractCalls = 0;

		await expect(
			installBinaryWithDependencies({
				supportedPlatform: {
					targetTriple: "x86_64-unknown-linux-gnu",
					os: "linux",
					arch: "x64",
				},
				version: "0.1.0",
				loadReleaseManifest: () =>
					loadVerifiedReleaseManifest("0.1.0", {
						downloadManifest: async () => manifestPayload,
						downloadManifestSignature: async () => signature,
						publicKeyPem: publicKey,
					}),
				downloadArchive: async (_url, destinationPath) => {
					writeFileSync(destinationPath, archiveContents);
				},
				checksumFile: async () => "mismatch",
				extractArchiveFn: () => {
					extractCalls += 1;
				},
			}),
		).rejects.toThrow("Downloaded sce archive checksum mismatch");

		expect(extractCalls).toBe(0);
	});

	test("installs after verifying the manifest signature and archive checksum", async () => {
		const version = "0.1.0";
		const targetTriple = "x86_64-unknown-linux-gnu";
		const archiveContents = "verified-archive";
		const checksum = createHash("sha256").update(archiveContents).digest("hex");
		const { manifestPayload, publicKey, signature } = createManifestFixture({
			artifacts: [{ target_triple: targetTriple, checksum_sha256: checksum }],
		});
		const tempDir = path.join(
			tmpdir(),
			`sce-npm-install-test-${Date.now()}-${Math.random()}`,
		);
		const installedBinaryPaths = [];
		const chmodCalls = [];
		tempDirs.push(tempDir);

		await expect(
			installBinaryWithDependencies({
				supportedPlatform: { targetTriple, os: "linux", arch: "x64" },
				version,
				loadReleaseManifest: () =>
					loadVerifiedReleaseManifest(version, {
						downloadManifest: async () => manifestPayload,
						downloadManifestSignature: async () => signature,
						publicKeyPem: publicKey,
					}),
				createTempDir: () => {
					mkdirSync(tempDir, { recursive: true });
					return tempDir;
				},
				removeTempDir: () => {},
				downloadArchive: async (_url, destinationPath) => {
					writeFileSync(destinationPath, archiveContents);
				},
				checksumFile: async () => checksum,
				extractArchiveFn: (_archivePath, destinationDir) => {
					const extractedBinaryPath = path.join(
						destinationDir,
						`sce-v${version}-${targetTriple}`,
						"bin",
						"sce",
					);
					mkdirSync(path.dirname(extractedBinaryPath), { recursive: true });
					writeFileSync(extractedBinaryPath, "#!/usr/bin/env bash\nexit 0\n");
				},
				fileExists: existsSync,
				createRuntimeDir: () => {},
				copyBinary: (sourcePath, destinationPath) => {
					installedBinaryPaths.push({ sourcePath, destinationPath });
				},
				chmodBinary: (destinationPath, mode) => {
					chmodCalls.push({ destinationPath, mode });
				},
			}),
		).resolves.toBeUndefined();

		expect(installedBinaryPaths).toHaveLength(1);
		expect(installedBinaryPaths[0].sourcePath).toBe(
			path.join(tempDir, `sce-v${version}-${targetTriple}`, "bin", "sce"),
		);
		expect(installedBinaryPaths[0].destinationPath).toBe(
			getInstalledBinaryPath(path.join(process.cwd(), "lib")),
		);
		expect(chmodCalls).toContainEqual({
			destinationPath: installedBinaryPaths[0].destinationPath,
			mode: 0o755,
		});
	});
});
