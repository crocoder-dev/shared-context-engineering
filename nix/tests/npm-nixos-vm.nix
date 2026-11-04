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

  rebuildScenarioDir = "${homeDir}/scenarios/rebuild";
  rebuildPrefixDir = "${rebuildScenarioDir}/prefix";
  rebuildFixtureDir = "${rebuildScenarioDir}/fixture";
  rebuildPortFile = "${rebuildScenarioDir}/server-port";
  rebuildPidFile = "${rebuildScenarioDir}/server.pid";
  rebuildInstalledDir = "${rebuildPrefixDir}/node_modules/@crocoder-dev/sce";
  rebuildLauncher = "${rebuildPrefixDir}/node_modules/.bin/sce";

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

  # Runs as the test user with HOME, npm cache/config, and TMPDIR isolated
  # under <scenarioDir>.
  asScenarioUser =
    scenarioDir: command:
    asUser ''
      export HOME=${scenarioDir}/home
      export TMPDIR=${scenarioDir}/tmp
      export npm_config_cache=${scenarioDir}/npm-cache
      export npm_config_userconfig=${scenarioDir}/npmrc
      ${command}
    '';
in
pkgs.testers.runNixOSTest {
  name = "npm-nixos-vm";

  nodes.machine = {
    networking.useDHCP = false;
    networking.interfaces = { };
    virtualisation.vlans = [ ];

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
        print(machine.succeed(${asScenarioUser rebuildScenarioDir ''
          . ${rebuildFixtureDir}/fixture.env
          SCE_NPM_RELEASE_BASE_URL="https://127.0.0.1:$(cat ${rebuildPortFile})" \
            NODE_EXTRA_CA_CERTS="$FIXTURE_TLS_CERT" \
            npm rebuild @crocoder-dev/sce \
            --prefix ${rebuildPrefixDir} \
            --offline \
            --foreground-scripts 2>&1
        ''}))
        runtime_mode = machine.succeed("stat -c %a ${rebuildInstalledDir}/runtime/sce").strip()
        print(f"{attempt}: runtime/sce mode: {runtime_mode}")
        assert runtime_mode == "755", f"{attempt}: runtime/sce mode {runtime_mode}, expected 755"
        machine.succeed(${asScenarioUser rebuildScenarioDir "${rebuildLauncher} --help >/dev/null"})
        reported_version = machine.succeed(
            ${asScenarioUser rebuildScenarioDir "${rebuildLauncher} version --format json | jq -er .version"}
        ).strip()
        print(f"{attempt}: parsed version: {reported_version}")
        assert reported_version == ${builtins.toJSON expectedVersion}, f"{attempt}: sce version reported {reported_version!r}"

    with subtest("skipped postinstall and npm rebuild recovery"):
        print(machine.succeed(${asScenarioUser rebuildScenarioDir ''
          mkdir -p "$HOME" "$TMPDIR" "$npm_config_cache"
          : >"$npm_config_userconfig"
          bash ${npmDir}/test/release-fixture.sh prepare ${rebuildFixtureDir} \
            --package-dir ${npmDir} \
            --binary ${sceReleasePackage}/bin/sce 2>&1
        ''}))
        machine.succeed(${asScenarioUser rebuildScenarioDir ''
          setsid bash ${npmDir}/test/release-fixture.sh serve ${rebuildFixtureDir} \
            --port-file ${rebuildPortFile} \
            >${rebuildScenarioDir}/server.log 2>&1 </dev/null &
          echo $! >${rebuildPidFile}
          # No explicit `exit`: it runs /etc/bash_logout, which fails under `set -u`.
          for _ in $(seq 100); do
            [ -s ${rebuildPortFile} ] && break
            sleep 0.1
          done
          [ -s ${rebuildPortFile} ] || {
            cat ${rebuildScenarioDir}/server.log >&2
            false
          }
        ''})
        try:
            print(machine.succeed(${asScenarioUser rebuildScenarioDir ''
              . ${rebuildFixtureDir}/fixture.env
              npm install \
                --prefix ${rebuildPrefixDir} \
                --offline \
                --no-audit \
                --no-fund \
                --ignore-scripts \
                "$FIXTURE_TARBALL" 2>&1
            ''}))
            machine.fail("test -e ${rebuildInstalledDir}/runtime/sce")
            print("ignore-scripts install: runtime/sce is absent")
            status, output = machine.execute(${asScenarioUser rebuildScenarioDir "${rebuildLauncher} --help 2>&1"})
            print(f"launcher without binary exited {status}: {output.strip()}")
            assert status != 0, "launcher without runtime/sce exited 0"
            assert ${builtins.toJSON missingBinaryMessage} in output, f"launcher output lacks the missing-binary message: {output!r}"

            rebuild_and_assert("first rebuild")
            rebuild_and_assert("second rebuild")
        finally:
            machine.succeed(${asScenarioUser rebuildScenarioDir ''
              kill "$(cat ${rebuildPidFile})" 2>/dev/null || true
            ''})
  '';
}
