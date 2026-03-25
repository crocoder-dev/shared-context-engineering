import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createWriteStream } from "node:fs";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
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
const RUNTIME_DIR = path.join(PACKAGE_ROOT, "runtime");

function readPackageVersion() {
  const packageJson = JSON.parse(readFileSync(PACKAGE_JSON_PATH, "utf8"));

  if (!packageJson.version || typeof packageJson.version !== "string") {
    throw new Error("Invalid npm package metadata: missing version.");
  }

  return packageJson.version;
}

function getReleaseBaseUrl(version) {
  return process.env.SCE_NPM_RELEASE_BASE_URL
    ?? `https://github.com/crocoder-dev/sce/releases/download/v${version}`;
}

function getManifestUrl(version) {
  return `${getReleaseBaseUrl(version)}/${getReleaseManifestName(version)}`;
}

async function downloadToFile(url, destinationPath, redirectsRemaining = 5) {
  await new Promise((resolve, reject) => {
    const request = get(url, (response) => {
      const { statusCode = 0, headers } = response;

      if (statusCode >= 300 && statusCode < 400 && headers.location) {
        response.resume();

        if (redirectsRemaining <= 0) {
          reject(new Error(`Too many redirects while downloading ${url}.`));
          return;
        }

        const redirectedUrl = new URL(headers.location, url).toString();
        downloadToFile(redirectedUrl, destinationPath, redirectsRemaining - 1)
          .then(resolve)
          .catch(reject);
        return;
      }

      if (statusCode !== 200) {
        response.resume();
        reject(new Error(`Unexpected response ${statusCode} while downloading ${url}.`));
        return;
      }

      const output = createWriteStream(destinationPath);
      pipeline(response, output).then(resolve).catch(reject);
    });

    request.on("error", reject);
  });
}

async function downloadJson(url) {
  const tempDir = mkdtempSync(path.join(tmpdir(), "sce-npm-json-"));
  const jsonPath = path.join(tempDir, "manifest.json");

  try {
    await downloadToFile(url, jsonPath);
    return JSON.parse(readFileSync(jsonPath, "utf8"));
  } finally {
    rmSync(tempDir, { recursive: true, force: true });
  }
}

function sha256File(filePath) {
  const hash = createHash("sha256");
  hash.update(readFileSync(filePath));
  return hash.digest("hex");
}

function extractArchive(archivePath, destinationDir) {
  const tarResult = spawnSync("tar", ["-xzf", archivePath, "-C", destinationDir], {
    stdio: "pipe",
    encoding: "utf8",
  });

  if (tarResult.status !== 0) {
    throw new Error(tarResult.stderr.trim() || "Failed to extract sce release archive.");
  }
}

export async function installBinary() {
  if (process.env.SCE_NPM_SKIP_DOWNLOAD === "1") {
    console.log("Skipping sce binary download because SCE_NPM_SKIP_DOWNLOAD=1.");
    return;
  }

  const supportedPlatform = resolveSupportedPlatform();
  const unsupportedMessage = formatUnsupportedPlatformMessage();

  if (!supportedPlatform) {
    throw new Error(unsupportedMessage ?? "Unsupported platform for npm sce package.");
  }

  const version = readPackageVersion();
  const releaseManifest = await downloadJson(getManifestUrl(version));
  const artifact = selectReleaseArtifact(releaseManifest, supportedPlatform.targetTriple);
  const archiveName = artifact.archive ?? getArchiveName(version, supportedPlatform.targetTriple);
  const expectedChecksum = artifact.checksum_sha256;

  if (!expectedChecksum) {
    throw new Error(`Release artifact ${archiveName} is missing checksum_sha256 metadata.`);
  }

  const tempDir = mkdtempSync(path.join(tmpdir(), "sce-npm-install-"));

  try {
    const archivePath = path.join(tempDir, archiveName);
    const archiveUrl = `${getReleaseBaseUrl(version)}/${archiveName}`;

    await downloadToFile(archiveUrl, archivePath);

    const actualChecksum = sha256File(archivePath);
    if (actualChecksum !== expectedChecksum) {
      throw new Error(
        `Downloaded sce archive checksum mismatch for ${archiveName}: expected ${expectedChecksum}, received ${actualChecksum}.`,
      );
    }

    extractArchive(archivePath, tempDir);

    const extractedBinaryPath = path.join(
      tempDir,
      getArchiveRoot(version, supportedPlatform.targetTriple),
      "bin",
      "sce",
    );

    if (!existsSync(extractedBinaryPath)) {
      throw new Error(`Extracted sce archive did not contain ${archiveName} -> bin/sce.`);
    }

    mkdirSync(RUNTIME_DIR, { recursive: true });
    const installedBinaryPath = getInstalledBinaryPath(__dirname);
    copyFileSync(extractedBinaryPath, installedBinaryPath);
    chmodSync(installedBinaryPath, 0o755);
  } finally {
    rmSync(tempDir, { recursive: true, force: true });
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  installBinary().catch((error) => {
    console.error(`Failed to install sce via npm: ${error.message}`);
    process.exit(1);
  });
}
