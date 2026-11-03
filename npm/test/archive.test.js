import { afterEach, describe, expect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import {
	chmodSync,
	mkdirSync,
	mkdtempSync,
	readdirSync,
	readFileSync,
	rmSync,
	statSync,
	writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { gzipSync } from "node:zlib";
import {
	ARCHIVE_LIMITS,
	extractReleaseBinary,
	installBinaryAtomically,
	readReleaseBinaryFromArchive,
} from "../lib/archive.js";
import {
	createReleaseEntries,
	createTarArchive,
	createTarEntry,
	createTarHeader,
} from "./tar-fixture.js";

const ARCHIVE_ROOT = "sce-v0.1.0-x86_64-unknown-linux-musl";
const BINARY_CONTENT = "#!/bin/sh\necho fixture-sce\n";

const tempDirs = [];

afterEach(() => {
	for (const tempDir of tempDirs.splice(0)) {
		rmSync(tempDir, { recursive: true, force: true });
	}
});

function createTempDir() {
	const tempDir = mkdtempSync(path.join(tmpdir(), "sce-npm-archive-test-"));
	tempDirs.push(tempDir);
	return tempDir;
}

function writeArchive(entries, options) {
	const tempDir = createTempDir();
	const archivePath = path.join(tempDir, `${ARCHIVE_ROOT}.tar.gz`);
	writeFileSync(archivePath, createTarArchive(entries, options));
	return { tempDir, archivePath };
}

function expectExtractionRejected(entries, message, limits) {
	const { tempDir, archivePath } = writeArchive(entries);
	const stagingDir = path.join(tempDir, "staging");
	mkdirSync(stagingDir);

	expect(() =>
		extractReleaseBinary(archivePath, {
			archiveRoot: ARCHIVE_ROOT,
			stagingDir,
			limits,
		}),
	).toThrow(message);

	expect(readdirSync(stagingDir)).toEqual([]);
	expect(readdirSync(tempDir).sort()).toEqual(
		[`${ARCHIVE_ROOT}.tar.gz`, "staging"].sort(),
	);
}

function releaseEntriesWith(extraEntries) {
	return [...createReleaseEntries(ARCHIVE_ROOT), ...extraEntries];
}

describe("release archive extraction", () => {
	test("extracts only bin/sce from a canonical archive into staging", () => {
		const { tempDir, archivePath } = writeArchive(
			createReleaseEntries(ARCHIVE_ROOT, { binaryContent: BINARY_CONTENT }),
		);
		const stagingDir = path.join(tempDir, "staging");
		mkdirSync(stagingDir);

		const stagedBinaryPath = extractReleaseBinary(archivePath, {
			archiveRoot: ARCHIVE_ROOT,
			stagingDir,
		});

		expect(stagedBinaryPath).toBe(path.join(stagingDir, "sce"));
		expect(readFileSync(stagedBinaryPath, "utf8")).toBe(BINARY_CONTENT);
		expect(readdirSync(stagingDir)).toEqual(["sce"]);
	});

	test("accepts an archive produced by GNU tar in the release layout", () => {
		const sourceDir = createTempDir();
		const rootDir = path.join(sourceDir, ARCHIVE_ROOT);
		mkdirSync(path.join(rootDir, "bin"), { recursive: true });
		writeFileSync(path.join(rootDir, "bin", "sce"), BINARY_CONTENT);
		chmodSync(path.join(rootDir, "bin", "sce"), 0o755);
		writeFileSync(path.join(rootDir, "LICENSE"), "MIT\n");
		writeFileSync(path.join(rootDir, "README.md"), "# sce\n");
		const archivePath = path.join(sourceDir, `${ARCHIVE_ROOT}.tar.gz`);
		const tarResult = spawnSync(
			"tar",
			[
				"--sort=name",
				"--owner=0",
				"--group=0",
				"--numeric-owner",
				"-C",
				sourceDir,
				"-czf",
				archivePath,
				ARCHIVE_ROOT,
			],
			{ encoding: "utf8" },
		);

		if (tarResult.error || tarResult.status !== 0) {
			console.warn(
				`Skipping GNU tar compatibility assertion: ${tarResult.error?.message ?? tarResult.stderr}`,
			);
			return;
		}

		expect(
			readReleaseBinaryFromArchive(archivePath, {
				archiveRoot: ARCHIVE_ROOT,
			}).toString("utf8"),
		).toBe(BINARY_CONTENT);
	});

	test("accepts ustar prefix-split names and GNU long-name headers", () => {
		const longNamePayload = `${ARCHIVE_ROOT}/README.md`;
		const entries = [
			{ name: `${ARCHIVE_ROOT}/`, type: "5", mode: 0o755 },
			{
				name: "LICENSE",
				prefix: ARCHIVE_ROOT,
				format: "ustar",
				content: "MIT\n",
			},
			{ name: "././@LongLink", type: "L", content: `${longNamePayload}\0` },
			{ name: "truncated-name", content: "# sce\n" },
			{ name: `${ARCHIVE_ROOT}/bin/`, type: "5", mode: 0o755 },
			{ name: `${ARCHIVE_ROOT}/bin/sce`, content: BINARY_CONTENT },
		];
		const { archivePath } = writeArchive(entries);

		expect(
			readReleaseBinaryFromArchive(archivePath, {
				archiveRoot: ARCHIVE_ROOT,
			}).toString("utf8"),
		).toBe(BINARY_CONTENT);
	});

	test("rejects an absolute entry path", () => {
		expectExtractionRejected(
			releaseEntriesWith([{ name: "/etc/passwd", content: "x" }]),
			'Invalid sce release archive: entry "/etc/passwd" has an absolute path.',
		);
	});

	test("rejects a .. traversal entry", () => {
		expectExtractionRejected(
			releaseEntriesWith([{ name: `${ARCHIVE_ROOT}/../escape`, content: "x" }]),
			`Invalid sce release archive: entry "${ARCHIVE_ROOT}/../escape" contains a ".." path segment.`,
		);
	});

	test("rejects a .. traversal hidden in a GNU long name", () => {
		expectExtractionRejected(
			[
				{ name: "././@LongLink", type: "L", content: "../../escape\0" },
				{ name: "innocent", content: "x" },
				...createReleaseEntries(ARCHIVE_ROOT),
			],
			'Invalid sce release archive: entry "../../escape" contains a ".." path segment.',
		);
	});

	test("rejects a symlink entry", () => {
		expectExtractionRejected(
			[
				...createReleaseEntries(ARCHIVE_ROOT).slice(0, 4),
				{ name: `${ARCHIVE_ROOT}/bin/sce`, type: "2", linkName: "/bin/sh" },
			],
			`Invalid sce release archive: entry "${ARCHIVE_ROOT}/bin/sce" is an unsupported symlink.`,
		);
	});

	test("rejects a hardlink entry", () => {
		expectExtractionRejected(
			[
				...createReleaseEntries(ARCHIVE_ROOT).slice(0, 4),
				{
					name: `${ARCHIVE_ROOT}/bin/sce`,
					type: "1",
					linkName: `${ARCHIVE_ROOT}/LICENSE`,
				},
			],
			`Invalid sce release archive: entry "${ARCHIVE_ROOT}/bin/sce" is an unsupported hardlink.`,
		);
	});

	test("rejects a duplicate entry", () => {
		expectExtractionRejected(
			releaseEntriesWith([
				{ name: `${ARCHIVE_ROOT}/bin/sce`, content: "#!/bin/sh\nevil\n" },
			]),
			`Invalid sce release archive: duplicate entry "${ARCHIVE_ROOT}/bin/sce".`,
		);
	});

	for (const [label, type] of [
		["FIFO", "6"],
		["character device", "3"],
		["block device", "4"],
	]) {
		test(`rejects an unexpected ${label} entry`, () => {
			expectExtractionRejected(
				releaseEntriesWith([{ name: `${ARCHIVE_ROOT}/device`, type }]),
				`Invalid sce release archive: entry "${ARCHIVE_ROOT}/device" is an unsupported ${label}.`,
			);
		});
	}

	test("rejects an entry outside the archive root", () => {
		expectExtractionRejected(
			releaseEntriesWith([{ name: "other-root/bin/sce", content: "x" }]),
			`Invalid sce release archive: entry "other-root/bin/sce" is outside the archive root "${ARCHIVE_ROOT}".`,
		);
	});

	test("rejects an unexpected entry inside the archive root", () => {
		expectExtractionRejected(
			releaseEntriesWith([{ name: `${ARCHIVE_ROOT}/bin/extra`, content: "x" }]),
			`Invalid sce release archive: unexpected entry "${ARCHIVE_ROOT}/bin/extra".`,
		);
	});

	test("rejects an oversize bin/sce payload", () => {
		expectExtractionRejected(
			createReleaseEntries(ARCHIVE_ROOT, { binaryContent: "x".repeat(2048) }),
			"Invalid sce release archive: bin/sce size 2048 exceeds limit 1024 bytes.",
			{ ...ARCHIVE_LIMITS, maxBinaryBytes: 1024 },
		);
	});

	test("rejects an archive whose uncompressed size exceeds the limit", () => {
		expectExtractionRejected(
			createReleaseEntries(ARCHIVE_ROOT, {
				binaryContent: "x".repeat(64 * 1024),
			}),
			"Invalid sce release archive: uncompressed size exceeds limit 16384 bytes.",
			{ ...ARCHIVE_LIMITS, maxUncompressedBytes: 16 * 1024 },
		);
	});

	test("rejects an archive whose compressed size exceeds the limit", () => {
		expectExtractionRejected(
			createReleaseEntries(ARCHIVE_ROOT),
			/Invalid sce release archive: compressed size \d+ exceeds limit 16 bytes\./,
			{ ...ARCHIVE_LIMITS, maxCompressedBytes: 16 },
		);
	});

	test("rejects an archive with too many entries", () => {
		expectExtractionRejected(
			createReleaseEntries(ARCHIVE_ROOT),
			"Invalid sce release archive: more than 3 entries.",
			{ ...ARCHIVE_LIMITS, maxEntries: 3 },
		);
	});

	test("rejects an archive missing bin/sce", () => {
		expectExtractionRejected(
			createReleaseEntries(ARCHIVE_ROOT).slice(0, 4),
			`Invalid sce release archive: missing regular file ${ARCHIVE_ROOT}/bin/sce.`,
		);
	});

	test("rejects a non-regular bin/sce", () => {
		expectExtractionRejected(
			[
				...createReleaseEntries(ARCHIVE_ROOT).slice(0, 4),
				{ name: `${ARCHIVE_ROOT}/bin/sce/`, type: "5" },
			],
			`Invalid sce release archive: entry "${ARCHIVE_ROOT}/bin/sce" must be a regular file.`,
		);
	});

	test("rejects a header with a corrupted checksum", () => {
		const header = createTarHeader({ name: `${ARCHIVE_ROOT}/`, type: "5" });
		header[0] ^= 0x01;

		expectExtractionRejected(
			[header],
			"Invalid sce release archive: tar header checksum mismatch.",
		);
	});

	test("rejects a truncated tar stream", () => {
		const { tempDir } = writeArchive([]);
		const archivePath = path.join(tempDir, "truncated.tar.gz");
		writeFileSync(
			archivePath,
			gzipSync(
				createTarEntry({ name: `${ARCHIVE_ROOT}/`, type: "5" }).subarray(
					0,
					512,
				),
			),
		);

		expect(() =>
			readReleaseBinaryFromArchive(archivePath, { archiveRoot: ARCHIVE_ROOT }),
		).toThrow("Invalid sce release archive: truncated tar stream");
	});

	test("rejects a non-gzip payload", () => {
		const { tempDir } = writeArchive([]);
		const archivePath = path.join(tempDir, "plain.tar.gz");
		writeFileSync(archivePath, "not a gzip stream");

		expect(() =>
			readReleaseBinaryFromArchive(archivePath, { archiveRoot: ARCHIVE_ROOT }),
		).toThrow("Invalid sce release archive: failed to decompress gzip stream");
	});
});

describe("atomic binary install", () => {
	function setupRuntime() {
		const tempDir = createTempDir();
		const runtimeDir = path.join(tempDir, "runtime");
		const sourcePath = path.join(tempDir, "staged-sce");
		const destinationPath = path.join(runtimeDir, "sce");
		mkdirSync(runtimeDir);
		writeFileSync(sourcePath, "#!/bin/sh\necho new\n");
		writeFileSync(destinationPath, "#!/bin/sh\necho old\n");
		chmodSync(destinationPath, 0o755);
		return { runtimeDir, sourcePath, destinationPath };
	}

	test("replaces the installed binary with mode 0755 and leaves no temp files", () => {
		const { runtimeDir, sourcePath, destinationPath } = setupRuntime();

		installBinaryAtomically(sourcePath, destinationPath);

		expect(readFileSync(destinationPath, "utf8")).toBe("#!/bin/sh\necho new\n");
		expect(statSync(destinationPath).mode & 0o777).toBe(0o755);
		expect(readdirSync(runtimeDir)).toEqual(["sce"]);
	});

	test("preserves the previous binary when interrupted before rename", () => {
		const { runtimeDir, sourcePath, destinationPath } = setupRuntime();

		expect(() =>
			installBinaryAtomically(sourcePath, destinationPath, {
				renameFile: () => {
					throw new Error("simulated interruption");
				},
			}),
		).toThrow("simulated interruption");

		expect(readFileSync(destinationPath, "utf8")).toBe("#!/bin/sh\necho old\n");
		expect(statSync(destinationPath).mode & 0o777).toBe(0o755);
		expect(readdirSync(runtimeDir)).toEqual(["sce"]);
	});
});
