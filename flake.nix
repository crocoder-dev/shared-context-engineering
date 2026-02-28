{
  description = "Development shell for Bun + TypeScript + Pkl";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
        };

        agnixLspShim = pkgs.writeShellScriptBin "agnix-lsp" ''
                    set -euo pipefail

                    if [ -n "''${AGNIX_LSP_BIN:-}" ] && [ -x "''${AGNIX_LSP_BIN}" ]; then
                      exec "''${AGNIX_LSP_BIN}" "$@"
                    fi

                    if [ -x "$HOME/.cargo/bin/agnix-lsp" ]; then
                      exec "$HOME/.cargo/bin/agnix-lsp" "$@"
                    fi

                    cat >&2 <<'EOF'
          agnix-lsp is not bundled in nixpkgs for this dev shell yet.

                    Manual fallback (non-automatic):
                      cargo install --locked agnix-lsp

                    Then either:
                      - ensure ~/.cargo/bin is on PATH, or
                      - set AGNIX_LSP_BIN to the agnix-lsp binary path.
          EOF
                    exit 1
        '';

      in
      {
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            bun
            jq
            pkl
            typescript
            nodePackages.typescript-language-server
            agnixLspShim
            cargo
            rustc
          ];

          shellHook = ''
            version_of() {
              "$1" --version 2>/dev/null | awk 'match($0, /[0-9]+(\.[0-9]+)+/) { print substr($0, RSTART, RLENGTH); exit }'
            }

            export PATH="$HOME/.cargo/bin:$PATH"

            if [ ! -x "$HOME/.cargo/bin/agnix" ]; then
              echo "- agnix: installing agnix-cli via cargo"
              cargo install --locked agnix-cli
            fi

            echo "- bun: $(version_of bun)"
            echo "- pkl: $(version_of pkl)"
            echo "- tsc: $(version_of tsc)"
            echo "- tsserver-lsp: $(version_of typescript-language-server)"
            echo "- rust: $(version_of rustc)"
            echo "- agnix: $(version_of agnix)"
          '';
        };
      }
    );
}
