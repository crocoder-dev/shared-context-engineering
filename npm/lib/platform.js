import path from "node:path";

const RELEASE_BINARY_NAME = "sce";
const SHA256_HEX_PATTERN = /^[0-9a-f]{64}$/;

const SUPPORTED_TARGETS = new Map([
	[
		"darwin:arm64",
		{ targetTriple: "aarch64-apple-darwin", os: "darwin", arch: "arm64" },
	],
	[
		"linux:arm64",
		{ targetTriple: "aarch64-unknown-linux-musl", os: "linux", arch: "arm64" },
	],
	[
		"linux:x64",
		{ targetTriple: "x86_64-unknown-linux-musl", os: "linux", arch: "x64" },
	],
]);

export function resolveSupportedPlatform(
	platform = process.platform,
	arch = process.arch,
) {
	return SUPPORTED_TARGETS.get(`${platform}:${arch}`) ?? null;
}

export function getArchiveRoot(version, targetTriple) {
	return `sce-v${version}-${targetTriple}`;
}

export function getArchiveName(version, targetTriple) {
	return `${getArchiveRoot(version, targetTriple)}.tar.gz`;
}

export function getReleaseManifestName(version) {
	return `sce-v${version}-release-manifest.json`;
}

export function getInstalledBinaryPath(baseDir) {
	return path.resolve(baseDir, "..", "runtime", "sce");
}

export function formatUnsupportedPlatformMessage(
	platform = process.platform,
	arch = process.arch,
) {
	if (resolveSupportedPlatform(platform, arch)) {
		return null;
	}

	return `The npm sce package currently supports darwin/arm64, linux/arm64, and linux/x64. Received ${platform}/${arch}.`;
}

export function selectReleaseArtifact(releaseManifest, targetTriple) {
	if (!releaseManifest || !Array.isArray(releaseManifest.artifacts)) {
		throw new Error("Invalid sce release manifest: missing artifacts array.");
	}

	const matches = releaseManifest.artifacts.filter(
		(candidate) => candidate?.target_triple === targetTriple,
	);

	if (matches.length === 0) {
		throw new Error(
			`No sce release artifact found for target ${targetTriple}.`,
		);
	}

	if (matches.length > 1) {
		throw new Error(
			`Invalid sce release manifest: found ${matches.length} artifacts for target ${targetTriple}; expected exactly one.`,
		);
	}

	return matches[0];
}

export function validateReleaseManifestIdentity(
	releaseManifest,
	{ version, targetTriple },
) {
	if (
		!releaseManifest ||
		typeof releaseManifest !== "object" ||
		Array.isArray(releaseManifest)
	) {
		throw new Error("Invalid sce release manifest: expected a JSON object.");
	}

	if (releaseManifest.version !== version) {
		throw new Error(
			`Invalid sce release manifest: version ${JSON.stringify(releaseManifest.version)} does not match npm package version ${JSON.stringify(version)}.`,
		);
	}

	if (releaseManifest.binary !== RELEASE_BINARY_NAME) {
		throw new Error(
			`Invalid sce release manifest: binary ${JSON.stringify(releaseManifest.binary)} is not ${JSON.stringify(RELEASE_BINARY_NAME)}.`,
		);
	}

	const artifact = selectReleaseArtifact(releaseManifest, targetTriple);
	const archiveName = getArchiveName(version, targetTriple);

	if (artifact.archive !== archiveName) {
		throw new Error(
			`Invalid sce release artifact for target ${targetTriple}: archive ${JSON.stringify(artifact.archive)} does not match expected ${JSON.stringify(archiveName)}.`,
		);
	}

	const expectedChecksum = artifact.checksum_sha256;

	if (
		typeof expectedChecksum !== "string" ||
		!SHA256_HEX_PATTERN.test(expectedChecksum)
	) {
		throw new Error(
			`Invalid sce release artifact ${archiveName}: checksum_sha256 must be 64 lowercase hexadecimal characters.`,
		);
	}

	return { artifact, archiveName, expectedChecksum };
}
