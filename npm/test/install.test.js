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

const FIXTURE_VERSION = "0.1.0";
const FIXTURE_TARGET_TRIPLE = "x86_64-unknown-linux-musl";
const FIXTURE_ARCHIVE_NAME = `sce-v${FIXTURE_VERSION}-${FIXTURE_TARGET_TRIPLE}.tar.gz`;
const FIXTURE_CHECKSUM = createHash("sha256")
	.update("fixture-archive")
	.digest("hex");
const FIXTURE_PLATFORM = {
	targetTriple: FIXTURE_TARGET_TRIPLE,
	os: "linux",
	arch: "x64",
};

function createArtifactFixture(overrides = {}) {
	return {
		version: FIXTURE_VERSION,
		binary: "sce",
		archive: FIXTURE_ARCHIVE_NAME,
		checksum_file: `${FIXTURE_ARCHIVE_NAME}.sha256`,
		checksum_sha256: FIXTURE_CHECKSUM,
		target_triple: FIXTURE_TARGET_TRIPLE,
		os: "linux",
		arch: "x64",
		...overrides,
	};
}

function createManifestFixture(overrides = {}) {
	const manifest = {
		version: FIXTURE_VERSION,
		binary: "sce",
		artifacts: [createArtifactFixture()],
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
					targetTriple: "x86_64-unknown-linux-musl",
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
					targetTriple: "x86_64-unknown-linux-musl",
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
			artifacts: [createArtifactFixture({ checksum_sha256: expectedChecksum })],
		});
		let extractCalls = 0;

		await expect(
			installBinaryWithDependencies({
				supportedPlatform: {
					targetTriple: "x86_64-unknown-linux-musl",
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
		const targetTriple = "x86_64-unknown-linux-musl";
		const archiveContents = "verified-archive";
		const checksum = createHash("sha256").update(archiveContents).digest("hex");
		const { manifestPayload, publicKey, signature } = createManifestFixture({
			artifacts: [createArtifactFixture({ checksum_sha256: checksum })],
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

describe("npm installer manifest identity validation", () => {
	async function expectRejectedBeforeDownload(manifestOverrides, message) {
		const { manifestPayload, publicKey, signature } =
			createManifestFixture(manifestOverrides);
		let archiveDownloads = 0;

		await expect(
			installBinaryWithDependencies({
				supportedPlatform: FIXTURE_PLATFORM,
				version: FIXTURE_VERSION,
				loadReleaseManifest: () =>
					loadVerifiedReleaseManifest(FIXTURE_VERSION, {
						downloadManifest: async () => manifestPayload,
						downloadManifestSignature: async () => signature,
						publicKeyPem: publicKey,
					}),
				createTempDir: () => {
					throw new Error("temp dir must not be created");
				},
				downloadArchive: async () => {
					archiveDownloads += 1;
				},
			}),
		).rejects.toThrow(message);

		expect(archiveDownloads).toBe(0);
	}

	test("rejects a manifest whose version differs from the package version", async () => {
		await expectRejectedBeforeDownload(
			{ version: "0.1.1" },
			'Invalid sce release manifest: version "0.1.1" does not match npm package version "0.1.0".',
		);
	});

	test("rejects a manifest whose binary is not sce", async () => {
		await expectRejectedBeforeDownload(
			{ binary: "not-sce" },
			'Invalid sce release manifest: binary "not-sce" is not "sce".',
		);
	});

	test("rejects a manifest with a missing binary field", async () => {
		await expectRejectedBeforeDownload(
			{ binary: undefined },
			'Invalid sce release manifest: binary undefined is not "sce".',
		);
	});

	test("rejects a manifest without an artifact for the current target", async () => {
		await expectRejectedBeforeDownload(
			{
				artifacts: [
					createArtifactFixture({ target_triple: "aarch64-apple-darwin" }),
				],
			},
			`No sce release artifact found for target ${FIXTURE_TARGET_TRIPLE}.`,
		);
	});

	test("rejects a manifest with duplicate artifacts for the current target", async () => {
		await expectRejectedBeforeDownload(
			{ artifacts: [createArtifactFixture(), createArtifactFixture()] },
			`Invalid sce release manifest: found 2 artifacts for target ${FIXTURE_TARGET_TRIPLE}; expected exactly one.`,
		);
	});

	test("rejects an artifact whose archive name is not canonical", async () => {
		await expectRejectedBeforeDownload(
			{ artifacts: [createArtifactFixture({ archive: "../evil.tar.gz" })] },
			`Invalid sce release artifact for target ${FIXTURE_TARGET_TRIPLE}: archive "../evil.tar.gz" does not match expected "${FIXTURE_ARCHIVE_NAME}".`,
		);
	});

	test("rejects an artifact with a missing archive name", async () => {
		await expectRejectedBeforeDownload(
			{ artifacts: [createArtifactFixture({ archive: undefined })] },
			`Invalid sce release artifact for target ${FIXTURE_TARGET_TRIPLE}: archive undefined does not match expected "${FIXTURE_ARCHIVE_NAME}".`,
		);
	});

	for (const [label, checksum] of [
		["short", "abc123"],
		["uppercase", FIXTURE_CHECKSUM.toUpperCase()],
		["non-hex", "g".repeat(64)],
		["missing", undefined],
	]) {
		test(`rejects an artifact with a ${label} checksum_sha256`, async () => {
			await expectRejectedBeforeDownload(
				{ artifacts: [createArtifactFixture({ checksum_sha256: checksum })] },
				`Invalid sce release artifact ${FIXTURE_ARCHIVE_NAME}: checksum_sha256 must be 64 lowercase hexadecimal characters.`,
			);
		});
	}

	test("downloads the canonical archive name for a valid manifest", async () => {
		const { manifestPayload, publicKey, signature } = createManifestFixture();
		const archiveUrls = [];

		await expect(
			installBinaryWithDependencies({
				supportedPlatform: FIXTURE_PLATFORM,
				version: FIXTURE_VERSION,
				loadReleaseManifest: () =>
					loadVerifiedReleaseManifest(FIXTURE_VERSION, {
						downloadManifest: async () => manifestPayload,
						downloadManifestSignature: async () => signature,
						publicKeyPem: publicKey,
					}),
				downloadArchive: async (url) => {
					archiveUrls.push(url);
				},
				checksumFile: async () => "mismatch",
			}),
		).rejects.toThrow("Downloaded sce archive checksum mismatch");

		expect(archiveUrls).toEqual([
			`${getReleaseBaseUrl(FIXTURE_VERSION)}/${FIXTURE_ARCHIVE_NAME}`,
		]);
	});
});
