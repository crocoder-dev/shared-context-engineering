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
  '';
}
