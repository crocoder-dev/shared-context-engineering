#!/usr/bin/env node

import { readFileSync, writeFileSync } from "node:fs";
import process from "node:process";

import {
  createReleaseManifestSignature,
  resolveReleaseManifestSigningKey,
} from "./lib/release-manifest-signing.mjs";

function usage() {
  process.stderr.write(`Usage: sign-release-manifest --manifest <path> --signature-output <path> [--private-key-file <path>]\n`);
}

function parseArgs(argv) {
  let manifestPath = "";
  let signatureOutputPath = "";
  let privateKeyPath = "";

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];

    switch (arg) {
      case "--manifest":
        manifestPath = argv[index + 1] ?? "";
        index += 1;
        break;
      case "--signature-output":
        signatureOutputPath = argv[index + 1] ?? "";
        index += 1;
        break;
      case "--private-key-file":
        privateKeyPath = argv[index + 1] ?? "";
        index += 1;
        break;
      case "--help":
      case "-h":
        usage();
        process.exit(0);
        break;
      default:
        throw new Error(`Unknown argument: ${arg}`);
    }
  }

  if (!manifestPath || !signatureOutputPath) {
    throw new Error("Both --manifest and --signature-output are required.");
  }

  return {
    manifestPath,
    signatureOutputPath,
    privateKeyPath,
  };
}

try {
  const { manifestPath, signatureOutputPath, privateKeyPath } = parseArgs(process.argv.slice(2));
  const manifestPayload = readFileSync(manifestPath);
  const privateKeyPem = resolveReleaseManifestSigningKey({ privateKeyPath });
  const signature = createReleaseManifestSignature(manifestPayload, privateKeyPem);

  writeFileSync(signatureOutputPath, `${signature}\n`, "utf8");
} catch (error) {
  usage();
  process.stderr.write(`${error.message}\n`);
  process.exit(1);
}
