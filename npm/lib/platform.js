import path from "node:path";

const SUPPORTED_TARGETS = new Map([
  ["darwin:arm64", { targetTriple: "aarch64-apple-darwin", os: "darwin", arch: "arm64" }],
  ["darwin:x64", { targetTriple: "x86_64-apple-darwin", os: "darwin", arch: "x64" }],
  ["linux:x64", { targetTriple: "x86_64-unknown-linux-gnu", os: "linux", arch: "x64" }],
]);

export function resolveSupportedPlatform(platform = process.platform, arch = process.arch) {
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

export function formatUnsupportedPlatformMessage(platform = process.platform, arch = process.arch) {
  if (resolveSupportedPlatform(platform, arch)) {
    return null;
  }

  return `The npm sce package currently supports darwin/arm64, darwin/x64, and linux/x64. Received ${platform}/${arch}.`;
}

export function selectReleaseArtifact(releaseManifest, targetTriple) {
  if (!releaseManifest || !Array.isArray(releaseManifest.artifacts)) {
    throw new Error("Invalid sce release manifest: missing artifacts array.");
  }

  const artifact = releaseManifest.artifacts.find((candidate) => candidate.target_triple === targetTriple);

  if (!artifact) {
    throw new Error(`No sce release artifact found for target ${targetTriple}.`);
  }

  return artifact;
}
