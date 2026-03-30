import { describe, expect, test } from "bun:test";

import {
	formatUnsupportedPlatformMessage,
	getArchiveName,
	getArchiveRoot,
	getReleaseManifestName,
	resolveSupportedPlatform,
	selectReleaseArtifact,
} from "../lib/platform.js";

describe("resolveSupportedPlatform", () => {
	test("maps supported darwin arm64", () => {
		expect(resolveSupportedPlatform("darwin", "arm64")).toEqual({
			targetTriple: "aarch64-apple-darwin",
			os: "darwin",
			arch: "arm64",
		});
	});

	test("maps supported linux x64", () => {
		expect(resolveSupportedPlatform("linux", "x64")).toEqual({
			targetTriple: "x86_64-unknown-linux-gnu",
			os: "linux",
			arch: "x64",
		});
	});

	test("maps supported linux arm64", () => {
		expect(resolveSupportedPlatform("linux", "arm64")).toEqual({
			targetTriple: "aarch64-unknown-linux-gnu",
			os: "linux",
			arch: "arm64",
		});
	});

	test("lists linux arm64 in unsupported platform guidance", () => {
		const unsupportedMessage = formatUnsupportedPlatformMessage(
			"win32",
			"arm64",
		);

		expect(unsupportedMessage).toContain("linux/arm64");
		expect(unsupportedMessage).toContain("win32/arm64");
	});

	test("returns null for unsupported platforms", () => {
		expect(resolveSupportedPlatform("win32", "arm64")).toBeNull();
		expect(formatUnsupportedPlatformMessage("win32", "arm64")).toContain(
			"win32/arm64",
		);
	});
});

describe("release naming helpers", () => {
	test("derive archive names from version and target triple", () => {
		expect(getArchiveRoot("0.1.0", "x86_64-unknown-linux-gnu")).toBe(
			"sce-v0.1.0-x86_64-unknown-linux-gnu",
		);
		expect(getArchiveName("0.1.0", "x86_64-unknown-linux-gnu")).toBe(
			"sce-v0.1.0-x86_64-unknown-linux-gnu.tar.gz",
		);
		expect(getReleaseManifestName("0.1.0")).toBe(
			"sce-v0.1.0-release-manifest.json",
		);
	});
});

describe("selectReleaseArtifact", () => {
	test("selects matching target triple", () => {
		const artifact = selectReleaseArtifact(
			{
				artifacts: [
					{ target_triple: "x86_64-apple-darwin", archive: "macos.tgz" },
					{ target_triple: "x86_64-unknown-linux-gnu", archive: "linux.tgz" },
				],
			},
			"x86_64-unknown-linux-gnu",
		);

		expect(artifact.archive).toBe("linux.tgz");
	});

	test("throws when target triple is absent", () => {
		expect(() =>
			selectReleaseArtifact(
				{
					artifacts: [
						{ target_triple: "x86_64-apple-darwin", archive: "macos.tgz" },
					],
				},
				"x86_64-unknown-linux-gnu",
			),
		).toThrow(
			"No sce release artifact found for target x86_64-unknown-linux-gnu.",
		);
	});
});
