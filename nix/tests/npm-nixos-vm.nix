{
  pkgs,
  npmSrc,
  sceReleasePackage,
  expectedVersion,
}:

let
  testUser = "sce-tester";
  homeDir = "/home/${testUser}";
  prefixDir = "${homeDir}/prefix";
  installedDir = "${prefixDir}/node_modules/@crocoder-dev/sce";
  launcher = "${prefixDir}/node_modules/.bin/sce";
  npmDir = "${homeDir}/npm";
  missingBinaryMessage = "The native sce binary is not installed.";

  authenticityFailureMessage = "Release manifest authenticity check failed";
  checksumFailureMessage = "Downloaded sce archive checksum mismatch";

  mkScenario = name: rec {
    dir = "${homeDir}/scenarios/${name}";
    prefixDir = "${dir}/prefix";
    fixtureDir = "${dir}/fixture";
    tmpDir = "${dir}/tmp";
    portFile = "${dir}/server-port";
    pidFile = "${dir}/server.pid";
    serverLog = "${dir}/server.log";
    installedDir = "${prefixDir}/node_modules/@crocoder-dev/sce";
    launcher = "${prefixDir}/node_modules/.bin/sce";
  };
  rebuildScenario = mkScenario "rebuild";
  reinstallScenario = mkScenario "reinstall";
  setupScenario = mkScenario "setup";
  minimalScenario = mkScenario "minimal";

  minimalNodeBinDir = "${minimalScenario.dir}/node-bin";
  minimalHomeDir = "${minimalScenario.dir}/minimal-home";
  absentRuntimeTools = [
    "tar"
    "curl"
    "gzip"
    "bash"
  ];
  fhsLoader = "/lib64/ld-linux-x86-64.so.2";
  readelf = "${pkgs.binutils-unwrapped}/bin/readelf";
  minimalRuntimeBinary = "${minimalScenario.installedDir}/runtime/sce";
  minimalLauncherEnv = "${pkgs.coreutils}/bin/env -i PATH=${minimalNodeBinDir} HOME=${minimalHomeDir}";
  minimalBinaryEnv = "${pkgs.coreutils}/bin/env -i PATH=/var/empty HOME=${minimalHomeDir}";

  setupRepoDir = "${setupScenario.dir}/repo";
  setupNestedDir = "${setupRepoDir}/nested/cwd";
  setupSentinel = "sce-tester sentinel: keep this line";
  setupToolsPath = pkgs.lib.makeBinPath [
    pkgs.nodejs
    pkgs.git
    pkgs.coreutils
  ];
  setupBaselineDirs = [
    "context/plans"
    "context/handovers"
    "context/decisions"
    "context/tmp"
  ];
  setupBaselineFiles = [
    "context/overview.md"
    "context/architecture.md"
    "context/patterns.md"
    "context/glossary.md"
    "context/context-map.md"
    "context/tmp/.gitignore"
  ];

  asUser =
    command:
    builtins.toJSON ''
      su - ${testUser} -s ${pkgs.bash}/bin/bash -c ${pkgs.lib.escapeShellArg ''
        set -euo pipefail
        export HOME=${homeDir}
        export TMPDIR=${homeDir}/tmp
        export npm_config_cache=${homeDir}/npm-cache
        export npm_config_userconfig=${homeDir}/npmrc
        ${command}
      ''}
    '';

  asScenarioUser =
    scenarioDir: command:
    asUser ''
      export HOME=${scenarioDir}/home
      export TMPDIR=${scenarioDir}/tmp
      export npm_config_cache=${scenarioDir}/npm-cache
      export npm_config_userconfig=${scenarioDir}/npmrc
      ${command}
    '';

  prepareScenarioFixture =
    scenario:
    asScenarioUser scenario.dir ''
      mkdir -p "$HOME" "$TMPDIR" "$npm_config_cache"
      : >"$npm_config_userconfig"
      bash ${npmDir}/test/release-fixture.sh prepare ${scenario.fixtureDir} \
        --package-dir ${npmDir} \
        --binary ${sceReleasePackage}/bin/sce 2>&1
    '';

  startScenarioServer =
    scenario:
    asScenarioUser scenario.dir ''
      setsid bash ${npmDir}/test/release-fixture.sh serve ${scenario.fixtureDir} \
        --port-file ${scenario.portFile} \
        >${scenario.serverLog} 2>&1 </dev/null &
      echo $! >${scenario.pidFile}
      # No explicit `exit`: it runs /etc/bash_logout, which fails under `set -u`.
      for _ in $(seq 100); do
        [ -s ${scenario.portFile} ] && break
        sleep 0.1
      done
      [ -s ${scenario.portFile} ] || {
        cat ${scenario.serverLog} >&2
        false
      }
    '';

  stopScenarioServer =
    scenario:
    asScenarioUser scenario.dir ''
      kill "$(cat ${scenario.pidFile})" 2>/dev/null || true
    '';

  installScenarioPackage =
    scenario:
    asScenarioUser scenario.dir ''
      . ${scenario.fixtureDir}/fixture.env
      SCE_NPM_RELEASE_BASE_URL="https://127.0.0.1:$(cat ${scenario.portFile})" \
        NODE_EXTRA_CA_CERTS="$FIXTURE_TLS_CERT" \
        npm install \
        --prefix ${scenario.prefixDir} \
        --offline \
        --no-audit \
        --no-fund \
        --foreground-scripts \
        "$FIXTURE_TARBALL" 2>&1
    '';

  asSetupUser =
    command:
    asScenarioUser setupScenario.dir ''
      export PATH=${setupToolsPath}
      export XDG_CONFIG_HOME="$HOME/.config"
      export XDG_STATE_HOME="$HOME/.local/state"
      export XDG_DATA_HOME="$HOME/.local/share"
      export XDG_CACHE_HOME="$HOME/.cache"
      export GIT_CONFIG_NOSYSTEM=1
      ${command}
    '';

  assertToolsAbsent =
    label: path:
    asScenarioUser minimalScenario.dir ''
      for tool in ${pkgs.lib.escapeShellArgs absentRuntimeTools}; do
        if PATH=${path} command -v "$tool" >/dev/null; then
          echo "${label}: $tool resolves on PATH=${path}" >&2
          false
        fi
      done
      echo "${label}: ${toString absentRuntimeTools} do not resolve on PATH=${path}"
    '';

  rebuildScenarioPackage =
    scenario:
    asScenarioUser scenario.dir ''
      . ${scenario.fixtureDir}/fixture.env
      SCE_NPM_RELEASE_BASE_URL="https://127.0.0.1:$(cat ${scenario.portFile})" \
        NODE_EXTRA_CA_CERTS="$FIXTURE_TLS_CERT" \
        npm rebuild @crocoder-dev/sce \
        --prefix ${scenario.prefixDir} \
        --offline \
        --foreground-scripts 2>&1
    '';
in
pkgs.testers.runNixOSTest {
  name = "npm-nixos-vm";

  nodes.machine = {
    networking.useDHCP = false;
    networking.interfaces = { };
    virtualisation.vlans = [ ];

    environment.stub-ld.enable = false;

    users.users.${testUser} = {
      isNormalUser = true;
      home = homeDir;
    };

    environment.systemPackages = [
      pkgs.bash
      pkgs.coreutils
      pkgs.gnutar
      pkgs.gzip
      pkgs.jq
      pkgs.nodejs
      pkgs.openssl
    ];

    virtualisation.memorySize = 2048;
  };

  testScript = ''
    machine.wait_for_unit("multi-user.target")

    with subtest("test user is non-root"):
        uid = machine.succeed("id -u ${testUser}").strip()
        assert uid != "0", f"test user must not be root, got uid {uid}"

    with subtest("stage npm package source"):
        machine.succeed(${asUser ''
          mkdir -p "$TMPDIR" "$npm_config_cache"
          : >"$npm_config_userconfig"
          cp -R ${npmSrc}/npm "$HOME/npm"
          chmod -R u+w "$HOME/npm"
        ''})

    with subtest("install the packed package through postinstall with the real binary"):
        print(machine.succeed(${asUser ''
          bash "$HOME/npm/test/pack-smoke.sh" \
            --binary ${sceReleasePackage}/bin/sce \
            --expected-version ${pkgs.lib.escapeShellArg expectedVersion} \
            --prefix ${prefixDir} 2>&1
        ''}))

    with subtest("installed modes"):
        runtime_mode = machine.succeed("stat -c %a ${installedDir}/runtime/sce").strip()
        print(f"runtime/sce mode: {runtime_mode}")
        assert runtime_mode == "755", f"runtime/sce mode {runtime_mode}, expected 755"
        launcher_mode = machine.succeed("stat -L -c %a ${launcher}").strip()
        print(f"launcher mode: {launcher_mode}")
        assert int(launcher_mode, 8) & 0o111, f"launcher mode {launcher_mode} is not executable"
        launcher_target = machine.succeed("realpath ${launcher}").strip()
        expected_target = machine.succeed("realpath ${installedDir}/bin/sce.js").strip()
        assert launcher_target == expected_target, f"launcher resolves to {launcher_target}, expected {expected_target}"

    with subtest("direct launcher execution"):
        machine.succeed(${asUser "${launcher} --help >/dev/null"})
        version_json = machine.succeed(${asUser "${launcher} version --format json"})
        print(f"sce version --format json: {version_json.strip()}")
        reported_version = machine.succeed(
            ${asUser "${launcher} version --format json | jq -er .version"}
        ).strip()
        print(f"parsed version: {reported_version}")
        assert reported_version == ${builtins.toJSON expectedVersion}, f"sce version reported {reported_version!r}"

    def rebuild_and_assert(attempt):
        print(machine.succeed(${rebuildScenarioPackage rebuildScenario}))
        runtime_mode = machine.succeed("stat -c %a ${rebuildScenario.installedDir}/runtime/sce").strip()
        print(f"{attempt}: runtime/sce mode: {runtime_mode}")
        assert runtime_mode == "755", f"{attempt}: runtime/sce mode {runtime_mode}, expected 755"
        machine.succeed(${asScenarioUser rebuildScenario.dir "${rebuildScenario.launcher} --help >/dev/null"})
        reported_version = machine.succeed(
            ${asScenarioUser rebuildScenario.dir "${rebuildScenario.launcher} version --format json | jq -er .version"}
        ).strip()
        print(f"{attempt}: parsed version: {reported_version}")
        assert reported_version == ${builtins.toJSON expectedVersion}, f"{attempt}: sce version reported {reported_version!r}"

    with subtest("skipped postinstall and npm rebuild recovery"):
        print(machine.succeed(${prepareScenarioFixture rebuildScenario}))
        machine.succeed(${startScenarioServer rebuildScenario})
        try:
            print(machine.succeed(${asScenarioUser rebuildScenario.dir ''
              . ${rebuildScenario.fixtureDir}/fixture.env
              npm install \
                --prefix ${rebuildScenario.prefixDir} \
                --offline \
                --no-audit \
                --no-fund \
                --ignore-scripts \
                "$FIXTURE_TARBALL" 2>&1
            ''}))
            machine.fail("test -e ${rebuildScenario.installedDir}/runtime/sce")
            print("ignore-scripts install: runtime/sce is absent")
            status, output = machine.execute(${asScenarioUser rebuildScenario.dir "${rebuildScenario.launcher} --help 2>&1"})
            print(f"launcher without binary exited {status}: {output.strip()}")
            assert status != 0, "launcher without runtime/sce exited 0"
            assert ${builtins.toJSON missingBinaryMessage} in output, f"launcher output lacks the missing-binary message: {output!r}"

            rebuild_and_assert("first rebuild")
            rebuild_and_assert("second rebuild")
        finally:
            machine.succeed(${stopScenarioServer rebuildScenario})

    def runtime_sha256():
        return machine.succeed(
            "sha256sum ${reinstallScenario.installedDir}/runtime/sce | cut -d ' ' -f 1"
        ).strip()

    def assert_binary_preserved(label, expected_sha256):
        actual_sha256 = runtime_sha256()
        print(f"{label}: runtime/sce sha256: {actual_sha256}")
        assert actual_sha256 == expected_sha256, f"{label}: runtime/sce sha256 {actual_sha256}, expected {expected_sha256}"
        runtime_mode = machine.succeed("stat -c %a ${reinstallScenario.installedDir}/runtime/sce").strip()
        print(f"{label}: runtime/sce mode: {runtime_mode}")
        assert runtime_mode == "755", f"{label}: runtime/sce mode {runtime_mode}, expected 755"
        machine.succeed(${asScenarioUser reinstallScenario.dir "${reinstallScenario.launcher} --help >/dev/null"})
        reported_version = machine.succeed(
            ${asScenarioUser reinstallScenario.dir "${reinstallScenario.launcher} version --format json | jq -er .version"}
        ).strip()
        print(f"{label}: parsed version: {reported_version}")
        assert reported_version == ${builtins.toJSON expectedVersion}, f"{label}: sce version reported {reported_version!r}"
        staging_dirs = machine.succeed(
            "find ${reinstallScenario.tmpDir} -mindepth 1 -maxdepth 1"
            " \\( -name 'sce-npm-install-*' -o -name 'sce-npm-text-*' \\) -print"
        ).strip()
        print(f"{label}: installer temp dirs in TMPDIR: {staging_dirs or 'none'}")
        assert staging_dirs == "", f"{label}: installer temp dirs left behind: {staging_dirs}"
        runtime_entries = machine.succeed("ls -A ${reinstallScenario.installedDir}/runtime").split()
        print(f"{label}: runtime/ entries: {runtime_entries}")
        assert runtime_entries == ["sce"], f"{label}: runtime/ holds {runtime_entries}, expected only sce"

    def assert_rebuild_fails(label, expected_message):
        status, output = machine.execute(${rebuildScenarioPackage reinstallScenario})
        print(output)
        print(f"{label}: npm rebuild exited {status}")
        assert status != 0, f"{label}: npm rebuild exited 0"
        assert expected_message in output, f"{label}: npm rebuild output lacks {expected_message!r}"
        print(f"{label}: failed with {expected_message!r}")

    with subtest("failed reinstall preserves existing binary"):
        print(machine.succeed(${prepareScenarioFixture reinstallScenario}))
        # Pristine copies of the served release files, restored after each tamper.
        machine.succeed(${asScenarioUser reinstallScenario.dir ''
          . ${reinstallScenario.fixtureDir}/fixture.env
          mkdir -p ${reinstallScenario.dir}/saved-release
          cp -pR "$FIXTURE_RELEASE_DIR"/. ${reinstallScenario.dir}/saved-release/
        ''})
        machine.succeed(${startScenarioServer reinstallScenario})
        try:
            print(machine.succeed(${installScenarioPackage reinstallScenario}))
            installed_sha256 = runtime_sha256()
            assert_binary_preserved("initial install", installed_sha256)

            # A well-formed signature over different bytes, so verification
            # (not decoding) is what rejects it.
            machine.succeed(${asScenarioUser reinstallScenario.dir ''
              . ${reinstallScenario.fixtureDir}/fixture.env
              printf 'not the release manifest\n' |
                openssl dgst -sha256 -sign ${reinstallScenario.fixtureDir}/keys/signing.pem |
                openssl base64 -A \
                  >"$FIXTURE_RELEASE_DIR/sce-v$FIXTURE_VERSION-release-manifest.json.sig"
            ''})
            assert_rebuild_fails("invalid manifest signature", ${builtins.toJSON authenticityFailureMessage})
            assert_binary_preserved("after invalid manifest signature", installed_sha256)

            machine.succeed(${asScenarioUser reinstallScenario.dir ''
              . ${reinstallScenario.fixtureDir}/fixture.env
              sig="sce-v$FIXTURE_VERSION-release-manifest.json.sig"
              cp -p ${reinstallScenario.dir}/saved-release/"$sig" "$FIXTURE_RELEASE_DIR/$sig"
              printf 'corrupted\n' >>"$FIXTURE_RELEASE_DIR/sce-v$FIXTURE_VERSION-$FIXTURE_TARGET_TRIPLE.tar.gz"
            ''})
            assert_rebuild_fails("archive checksum mismatch", ${builtins.toJSON checksumFailureMessage})
            assert_binary_preserved("after archive checksum mismatch", installed_sha256)

            machine.succeed(${asScenarioUser reinstallScenario.dir ''
              . ${reinstallScenario.fixtureDir}/fixture.env
              archive="sce-v$FIXTURE_VERSION-$FIXTURE_TARGET_TRIPLE.tar.gz"
              cp -p ${reinstallScenario.dir}/saved-release/"$archive" "$FIXTURE_RELEASE_DIR/$archive"
            ''})
            print(machine.succeed(${rebuildScenarioPackage reinstallScenario}))
            assert_binary_preserved("restored fixture reinstall", installed_sha256)
        finally:
            machine.succeed(${stopScenarioServer reinstallScenario})

    def setup_tree():
        return machine.succeed(
            "cd ${setupRepoDir} && find context -mindepth 1 | LC_ALL=C sort"
        ).split()

    def assert_setup_baseline(label):
        tree = setup_tree()
        print(f"{label}: context tree: {tree}")
        for path in ${builtins.toJSON setupBaselineDirs}:
            machine.succeed(f"test -d ${setupRepoDir}/{path}")
        for path in ${builtins.toJSON setupBaselineFiles}:
            machine.succeed(f"test -f ${setupRepoDir}/{path}")
        expected_tree = sorted(${builtins.toJSON (setupBaselineDirs ++ setupBaselineFiles)})
        assert tree == expected_tree, f"{label}: context tree {tree}, expected {expected_tree}"
        return tree

    with subtest("real sce setup --bootstrap-context"):
        print(machine.succeed(${prepareScenarioFixture setupScenario}))
        machine.succeed(${startScenarioServer setupScenario})
        try:
            print(machine.succeed(${installScenarioPackage setupScenario}))
        finally:
            machine.succeed(${stopScenarioServer setupScenario})

        print(machine.succeed(${asSetupUser ''
          echo "setup PATH: $PATH"
          for tool in tar curl gzip bash jq; do
            if command -v "$tool" >/dev/null; then
              echo "$tool resolves on the setup PATH" >&2
              false
            fi
          done
          git init --quiet ${setupRepoDir}
          git -C ${setupRepoDir} remote add origin https://example.invalid/sce-tester/setup.git
          mkdir -p ${setupNestedDir}
        ''}))

        print(machine.succeed(${asSetupUser ''
          cd ${setupRepoDir}
          echo "first run cwd: $(pwd)"
          ${setupScenario.launcher} setup --bootstrap-context 2>&1
        ''}))
        first_tree = assert_setup_baseline("first run")
        tmp_gitignore = machine.succeed("cat ${setupRepoDir}/context/tmp/.gitignore")
        assert tmp_gitignore == "*\n!.gitignore\n", f"context/tmp/.gitignore is {tmp_gitignore!r}"

        machine.succeed(${asSetupUser ''
          printf '%s\n' ${pkgs.lib.escapeShellArg setupSentinel} >>${setupRepoDir}/context/overview.md
        ''})
        overview_before = machine.succeed("cat ${setupRepoDir}/context/overview.md")

        print(machine.succeed(${asSetupUser ''
          cd ${setupNestedDir}
          echo "second run cwd: $(pwd)"
          ${setupScenario.launcher} setup --bootstrap-context 2>&1
        ''}))
        second_tree = assert_setup_baseline("second run")
        assert second_tree == first_tree, f"second run changed the context tree: {first_tree} -> {second_tree}"
        print("second run: context tree unchanged")
        overview_after = machine.succeed("cat ${setupRepoDir}/context/overview.md")
        sentinel = ${builtins.toJSON setupSentinel}
        assert sentinel in overview_after, f"sentinel lost from context/overview.md: {overview_after!r}"
        assert overview_after == overview_before, f"context/overview.md changed: {overview_before!r} -> {overview_after!r}"
        print(f"second run: sentinel preserved: {sentinel!r}")

        repo_root = machine.succeed(${asSetupUser "git -C ${setupNestedDir} rev-parse --show-toplevel"}).strip()
        assert repo_root == "${setupRepoDir}", f"nested cwd resolves to repo {repo_root!r}"
        repo_entries = machine.succeed("ls -A ${setupRepoDir}").split()
        print(f"repo entries: {repo_entries}")
        assert sorted(repo_entries) == [".git", "context", "nested"], f"repo holds {repo_entries}"
        for cwd in ["${setupRepoDir}", "${setupNestedDir}"]:
            assert not cwd.startswith("${npmDir}/"), f"cwd {cwd} is inside the staged npm source"
        machine.fail(
            "cd ${setupRepoDir}/.. && while :; do"
            " [ -e .git ] || [ -e cli/Cargo.toml ] && exit 0;"
            " [ \"$PWD\" = / ] && exit 1; cd ..; done"
        )
        machine.fail("grep -rlE '${npmDir}|/nix/store' ${setupRepoDir}/context")
        print("no SCE source path in the cwd ancestry or the bootstrapped context")

    def assert_minimal_version(label, version_json):
        print(f"{label}: version --format json: {version_json.strip()}")
        reported_version = json.loads(version_json)["version"]
        assert reported_version == ${builtins.toJSON expectedVersion}, f"{label}: sce version reported {reported_version!r}"
        print(f"{label}: parsed version: {reported_version}")

    with subtest("minimal runtime dependencies"):
        import json

        print(machine.succeed(${prepareScenarioFixture minimalScenario}))
        machine.succeed(${startScenarioServer minimalScenario})
        try:
            print(machine.succeed(${installScenarioPackage minimalScenario}))
        finally:
            machine.succeed(${stopScenarioServer minimalScenario})

        machine.succeed(${asScenarioUser minimalScenario.dir ''
          mkdir -p ${minimalNodeBinDir} ${minimalHomeDir}
          ln -s ${pkgs.nodejs}/bin/node ${minimalNodeBinDir}/node
        ''})
        node_bin_entries = machine.succeed("ls -A ${minimalNodeBinDir}").split()
        print(f"launcher PATH entries: {node_bin_entries}")
        assert node_bin_entries == ["node"], f"launcher PATH holds {node_bin_entries}, expected only node"
        print(machine.succeed(${assertToolsAbsent "launcher PATH" minimalNodeBinDir}))
        print(machine.succeed(${assertToolsAbsent "binary PATH" "/var/empty"}))

        machine.succeed("${readelf} -h ${minimalRuntimeBinary} >/dev/null")
        program_headers = machine.succeed("${readelf} -lW ${minimalRuntimeBinary}")
        assert "INTERP" not in program_headers and "program interpreter" not in program_headers, (
            f"runtime/sce requests an ELF interpreter:\n{program_headers}"
        )
        print("runtime/sce: no ELF INTERP program header")
        machine.fail("test -e ${fhsLoader}")
        print("no FHS loader at ${fhsLoader}")

        machine.succeed(${asScenarioUser minimalScenario.dir "${minimalLauncherEnv} ${minimalScenario.launcher} --help >/dev/null"})
        print("launcher: --help under env -i with only node on PATH")
        assert_minimal_version("launcher", machine.succeed(
            ${asScenarioUser minimalScenario.dir "${minimalLauncherEnv} ${minimalScenario.launcher} version --format json"}
        ))
        machine.succeed(${asScenarioUser minimalScenario.dir "${minimalBinaryEnv} ${minimalRuntimeBinary} --help >/dev/null"})
        print("runtime/sce: --help under env -i with PATH=/var/empty")
        assert_minimal_version("runtime/sce", machine.succeed(
            ${asScenarioUser minimalScenario.dir "${minimalBinaryEnv} ${minimalRuntimeBinary} version --format json"}
        ))
  '';
}
