import { readFileSync } from "node:fs";
import process from "node:process";
import { sign } from "node:crypto";

const SIGNING_KEY_ENV_VAR = "SCE_RELEASE_MANIFEST_SIGNING_KEY";

export function resolveReleaseManifestSigningKey({
  privateKeyPath,
  env = process.env,
} = {}) {
  const envPrivateKey = env[SIGNING_KEY_ENV_VAR];

  if (privateKeyPath && envPrivateKey) {
    throw new Error(
      `Provide either --private-key-file or ${SIGNING_KEY_ENV_VAR}, not both.`,
    );
  }

  if (privateKeyPath) {
    return readFileSync(privateKeyPath, "utf8");
  }

  if (typeof envPrivateKey === "string" && envPrivateKey.trim().length > 0) {
    return envPrivateKey;
  }

  throw new Error(
    `Release manifest signing key not provided. Set ${SIGNING_KEY_ENV_VAR} or pass --private-key-file <path>.`,
  );
}

export function createReleaseManifestSignature(manifestPayload, privateKeyPem) {
  if (!Buffer.isBuffer(manifestPayload) && !(manifestPayload instanceof Uint8Array)) {
    throw new TypeError("manifestPayload must be a Buffer or Uint8Array.");
  }

  const manifestBuffer = Buffer.isBuffer(manifestPayload)
    ? manifestPayload
    : Buffer.from(manifestPayload);

  return sign("sha256", manifestBuffer, privateKeyPem).toString("base64");
}

export { SIGNING_KEY_ENV_VAR };
