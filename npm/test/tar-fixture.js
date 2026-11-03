import { gzipSync } from "node:zlib";

const TAR_BLOCK_SIZE = 512;

function writeString(block, offset, length, value) {
	const bytes = Buffer.from(value, "utf8");

	if (bytes.length > length) {
		throw new Error(`tar fixture field too long: ${value}`);
	}

	bytes.copy(block, offset);
}

function writeOctal(block, offset, length, value) {
	writeString(
		block,
		offset,
		length,
		`${value.toString(8).padStart(length - 1, "0")}\0`,
	);
}

function padToBlock(buffer) {
	const remainder = buffer.length % TAR_BLOCK_SIZE;
	return remainder === 0
		? buffer
		: Buffer.concat([buffer, Buffer.alloc(TAR_BLOCK_SIZE - remainder)]);
}

export function createTarHeader({
	name,
	size = 0,
	type = "0",
	mode = 0o644,
	linkName = "",
	prefix = "",
	format = "gnu",
}) {
	const block = Buffer.alloc(TAR_BLOCK_SIZE);

	writeString(block, 0, 100, name);
	writeOctal(block, 100, 8, mode);
	writeOctal(block, 108, 8, 0);
	writeOctal(block, 116, 8, 0);
	writeOctal(block, 124, 12, size);
	writeOctal(block, 136, 12, 0);
	block.fill(0x20, 148, 156);
	writeString(block, 156, 1, type);
	writeString(block, 157, 100, linkName);

	if (format === "ustar") {
		writeString(block, 257, 8, "ustar\u000000");
		writeString(block, 345, 155, prefix);
	} else {
		writeString(block, 257, 8, "ustar  \u0000");
	}

	let checksum = 0;
	for (const byte of block) {
		checksum += byte;
	}
	writeString(block, 148, 8, `${checksum.toString(8).padStart(6, "0")}\0 `);

	return block;
}

export function createTarEntry({ content = "", ...header }) {
	const payload = Buffer.from(content);
	return Buffer.concat([
		createTarHeader({ ...header, size: payload.length }),
		padToBlock(payload),
	]);
}

export function createTarArchive(entries, { gzip = true } = {}) {
	const tarBuffer = Buffer.concat([
		...entries.map((entry) =>
			Buffer.isBuffer(entry) ? entry : createTarEntry(entry),
		),
		Buffer.alloc(TAR_BLOCK_SIZE * 2),
	]);

	return gzip ? gzipSync(tarBuffer) : tarBuffer;
}

export function createReleaseEntries(
	archiveRoot,
	{ binaryContent = "#!/bin/sh\necho sce\n" } = {},
) {
	return [
		{ name: `${archiveRoot}/`, type: "5", mode: 0o755 },
		{ name: `${archiveRoot}/LICENSE`, content: "MIT\n" },
		{ name: `${archiveRoot}/README.md`, content: "# sce\n" },
		{ name: `${archiveRoot}/bin/`, type: "5", mode: 0o755 },
		{
			name: `${archiveRoot}/bin/sce`,
			content: binaryContent,
			mode: 0o755,
		},
	];
}
