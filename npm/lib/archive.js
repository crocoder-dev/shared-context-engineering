import {
	closeSync,
	fchmodSync,
	fsyncSync,
	openSync,
	readFileSync,
	renameSync,
	rmSync,
	statSync,
	writeSync,
} from "node:fs";
import path from "node:path";
import { gunzipSync } from "node:zlib";

const TAR_BLOCK_SIZE = 512;
const MEBIBYTE = 1024 * 1024;

export const ARCHIVE_LIMITS = Object.freeze({
	maxCompressedBytes: 64 * MEBIBYTE,
	maxUncompressedBytes: 256 * MEBIBYTE,
	maxBinaryBytes: 192 * MEBIBYTE,
	maxEntries: 16,
	maxLongNameBytes: 4096,
});

const STAGED_BINARY_NAME = "sce";
const INSTALLED_BINARY_MODE = 0o755;
const STAGED_FILE_MODE = 0o600;

const ENTRY_KIND_DIRECTORY = "directory";
const ENTRY_KIND_FILE = "file";

const TYPE_REGULAR = "0";
const TYPE_REGULAR_LEGACY = "\0";
const TYPE_HARDLINK = "1";
const TYPE_SYMLINK = "2";
const TYPE_DIRECTORY = "5";
const TYPE_GNU_LONG_NAME = "L";
const TYPE_PAX_HEADER = "x";

const USTAR_MAGIC = "ustar\u000000";
const GNU_MAGIC = "ustar  \u0000";

function archiveError(message) {
	return new Error(`Invalid sce release archive: ${message}`);
}

function getAllowedEntries(archiveRoot) {
	return new Map([
		[archiveRoot, ENTRY_KIND_DIRECTORY],
		[`${archiveRoot}/bin`, ENTRY_KIND_DIRECTORY],
		[`${archiveRoot}/bin/sce`, ENTRY_KIND_FILE],
		[`${archiveRoot}/LICENSE`, ENTRY_KIND_FILE],
		[`${archiveRoot}/README.md`, ENTRY_KIND_FILE],
	]);
}

function readCString(block, offset, length) {
	const field = block.subarray(offset, offset + length);
	const end = field.indexOf(0);
	return field.subarray(0, end === -1 ? field.length : end).toString("utf8");
}

function readOctal(block, offset, length, fieldName) {
	const field = block.subarray(offset, offset + length);

	if (field[0] & 0x80) {
		throw archiveError(`unsupported base-256 ${fieldName} field.`);
	}

	const text = readCString(block, offset, length).trim();

	if (text.length === 0) {
		return 0;
	}

	if (!/^[0-7]+$/.test(text)) {
		throw archiveError(`malformed ${fieldName} field.`);
	}

	return Number.parseInt(text, 8);
}

function isZeroBlock(block) {
	return block.every((byte) => byte === 0);
}

function verifyHeaderChecksum(block) {
	const expected = readOctal(block, 148, 8, "checksum");
	let actual = 0;

	for (let index = 0; index < TAR_BLOCK_SIZE; index += 1) {
		actual += index >= 148 && index < 156 ? 0x20 : block[index];
	}

	if (actual !== expected) {
		throw archiveError("tar header checksum mismatch.");
	}
}

function parsePaxPath(payload) {
	let offset = 0;
	let paxPath;

	while (offset < payload.length) {
		const spaceIndex = payload.indexOf(0x20, offset);

		if (spaceIndex === -1) {
			throw archiveError("malformed pax header record.");
		}

		const recordLength = Number.parseInt(
			payload.subarray(offset, spaceIndex).toString("utf8"),
			10,
		);

		if (
			!Number.isSafeInteger(recordLength) ||
			recordLength <= spaceIndex - offset ||
			offset + recordLength > payload.length
		) {
			throw archiveError("malformed pax header record.");
		}

		const record = payload
			.subarray(spaceIndex + 1, offset + recordLength - 1)
			.toString("utf8");
		const equalsIndex = record.indexOf("=");

		if (equalsIndex === -1) {
			throw archiveError("malformed pax header record.");
		}

		const key = record.slice(0, equalsIndex);
		const value = record.slice(equalsIndex + 1);

		if (key === "path") {
			paxPath = value;
		} else if (key === "linkpath" || key === "size") {
			throw archiveError(`unsupported pax header key ${JSON.stringify(key)}.`);
		}

		offset += recordLength;
	}

	return paxPath;
}

function normalizeEntryName(rawName) {
	if (rawName.length === 0) {
		throw archiveError("entry with an empty name.");
	}

	if (rawName.startsWith("/") || /^[A-Za-z]:/.test(rawName)) {
		throw archiveError(
			`entry ${JSON.stringify(rawName)} has an absolute path.`,
		);
	}

	if (rawName.includes("\\")) {
		throw archiveError(
			`entry ${JSON.stringify(rawName)} contains a backslash.`,
		);
	}

	const name = rawName.endsWith("/") ? rawName.slice(0, -1) : rawName;
	const segments = name.split("/");

	if (segments.includes("..")) {
		throw archiveError(
			`entry ${JSON.stringify(rawName)} contains a ".." path segment.`,
		);
	}

	if (segments.some((segment) => segment === "" || segment === ".")) {
		throw archiveError(
			`entry ${JSON.stringify(rawName)} contains an empty or "." path segment.`,
		);
	}

	return name;
}

function describeEntryType(typeFlag) {
	switch (typeFlag) {
		case TYPE_HARDLINK:
			return "hardlink";
		case TYPE_SYMLINK:
			return "symlink";
		case "3":
			return "character device";
		case "4":
			return "block device";
		case "6":
			return "FIFO";
		default:
			return `type ${JSON.stringify(typeFlag)}`;
	}
}

function classifyEntryType(typeFlag, name) {
	if (typeFlag === TYPE_REGULAR || typeFlag === TYPE_REGULAR_LEGACY) {
		return ENTRY_KIND_FILE;
	}

	if (typeFlag === TYPE_DIRECTORY) {
		return ENTRY_KIND_DIRECTORY;
	}

	throw archiveError(
		`entry ${JSON.stringify(name)} is an unsupported ${describeEntryType(typeFlag)}.`,
	);
}

function gunzipArchive(archivePath, limits) {
	const { size } = statSync(archivePath);

	if (size > limits.maxCompressedBytes) {
		throw archiveError(
			`compressed size ${size} exceeds limit ${limits.maxCompressedBytes} bytes.`,
		);
	}

	try {
		return gunzipSync(readFileSync(archivePath), {
			maxOutputLength: limits.maxUncompressedBytes,
		});
	} catch (error) {
		if (error?.code === "ERR_BUFFER_TOO_LARGE") {
			throw archiveError(
				`uncompressed size exceeds limit ${limits.maxUncompressedBytes} bytes.`,
			);
		}

		throw archiveError(`failed to decompress gzip stream: ${error.message}`);
	}
}

export function readReleaseBinaryFromArchive(
	archivePath,
	{ archiveRoot, limits = ARCHIVE_LIMITS },
) {
	const tarBuffer = gunzipArchive(archivePath, limits);
	const allowedEntries = getAllowedEntries(archiveRoot);
	const binaryEntryName = `${archiveRoot}/bin/sce`;
	const seenEntries = new Set();
	let binaryPayload;
	let pendingLongName;
	let entryCount = 0;
	let offset = 0;

	while (true) {
		if (offset + TAR_BLOCK_SIZE > tarBuffer.length) {
			throw archiveError("truncated tar stream (missing end-of-archive).");
		}

		const header = tarBuffer.subarray(offset, offset + TAR_BLOCK_SIZE);
		offset += TAR_BLOCK_SIZE;

		if (isZeroBlock(header)) {
			if (pendingLongName !== undefined) {
				throw archiveError("long-name header without a following entry.");
			}
			break;
		}

		verifyHeaderChecksum(header);

		const magic = header.subarray(257, 265).toString("latin1");

		if (magic !== USTAR_MAGIC && magic !== GNU_MAGIC) {
			throw archiveError("entry header is not ustar or GNU tar format.");
		}

		const typeFlag = String.fromCharCode(header[156]);
		const size = readOctal(header, 124, 12, "size");
		const paddedSize = Math.ceil(size / TAR_BLOCK_SIZE) * TAR_BLOCK_SIZE;

		if (offset + paddedSize > tarBuffer.length) {
			throw archiveError("truncated tar entry payload.");
		}

		const payload = tarBuffer.subarray(offset, offset + size);
		offset += paddedSize;

		if (typeFlag === TYPE_GNU_LONG_NAME || typeFlag === TYPE_PAX_HEADER) {
			if (pendingLongName !== undefined) {
				throw archiveError("consecutive long-name headers.");
			}

			if (size > limits.maxLongNameBytes) {
				throw archiveError("long-name header exceeds size limit.");
			}

			pendingLongName =
				typeFlag === TYPE_GNU_LONG_NAME
					? readCString(payload, 0, payload.length)
					: (parsePaxPath(payload) ?? null);
			continue;
		}

		entryCount += 1;

		if (entryCount > limits.maxEntries) {
			throw archiveError(`more than ${limits.maxEntries} entries.`);
		}

		const prefix = magic === USTAR_MAGIC ? readCString(header, 345, 155) : "";
		const headerName = readCString(header, 0, 100);
		const rawName =
			pendingLongName ?? (prefix ? `${prefix}/${headerName}` : headerName);
		pendingLongName = undefined;

		const name = normalizeEntryName(rawName);
		const kind = classifyEntryType(typeFlag, name);

		if (name !== archiveRoot && !name.startsWith(`${archiveRoot}/`)) {
			throw archiveError(
				`entry ${JSON.stringify(name)} is outside the archive root ${JSON.stringify(archiveRoot)}.`,
			);
		}

		const allowedKind = allowedEntries.get(name);

		if (allowedKind === undefined) {
			throw archiveError(`unexpected entry ${JSON.stringify(name)}.`);
		}

		if (seenEntries.has(name)) {
			throw archiveError(`duplicate entry ${JSON.stringify(name)}.`);
		}
		seenEntries.add(name);

		if (kind !== allowedKind) {
			throw archiveError(
				`entry ${JSON.stringify(name)} must be a ${allowedKind === ENTRY_KIND_FILE ? "regular file" : "directory"}.`,
			);
		}

		if (name === binaryEntryName) {
			if (size > limits.maxBinaryBytes) {
				throw archiveError(
					`bin/sce size ${size} exceeds limit ${limits.maxBinaryBytes} bytes.`,
				);
			}

			binaryPayload = payload;
		}
	}

	if (binaryPayload === undefined) {
		throw archiveError(`missing regular file ${binaryEntryName}.`);
	}

	return binaryPayload;
}

function writeFileDurably(filePath, contents, mode) {
	const fd = openSync(filePath, "wx", STAGED_FILE_MODE);

	try {
		let written = 0;
		while (written < contents.length) {
			written += writeSync(fd, contents, written, contents.length - written);
		}
		fchmodSync(fd, mode);
		fsyncSync(fd);
	} finally {
		closeSync(fd);
	}
}

export function extractReleaseBinary(
	archivePath,
	{ archiveRoot, stagingDir, limits = ARCHIVE_LIMITS },
) {
	const binaryPayload = readReleaseBinaryFromArchive(archivePath, {
		archiveRoot,
		limits,
	});
	const stagedBinaryPath = path.join(stagingDir, STAGED_BINARY_NAME);

	writeFileDurably(stagedBinaryPath, binaryPayload, STAGED_FILE_MODE);

	return stagedBinaryPath;
}

function fsyncDirectoryBestEffort(directoryPath) {
	let fd;

	try {
		fd = openSync(directoryPath, "r");
		fsyncSync(fd);
	} catch {
		return;
	} finally {
		if (fd !== undefined) {
			closeSync(fd);
		}
	}
}

export function installBinaryAtomically(
	sourcePath,
	destinationPath,
	{ renameFile = renameSync } = {},
) {
	const destinationDir = path.dirname(destinationPath);
	const tempPath = path.join(
		destinationDir,
		`.${path.basename(destinationPath)}.${process.pid}.${Date.now()}.tmp`,
	);

	try {
		writeFileDurably(tempPath, readFileSync(sourcePath), INSTALLED_BINARY_MODE);
		renameFile(tempPath, destinationPath);
	} catch (error) {
		rmSync(tempPath, { force: true });
		throw error;
	}

	fsyncDirectoryBestEffort(destinationDir);
}
