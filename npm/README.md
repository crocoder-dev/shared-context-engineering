# sce

Thin npm launcher package for the `sce` CLI.

On install, this package downloads the matching platform release artifact for the
current `sce` version from GitHub Releases, verifies the published SHA-256
checksum, and installs the native `sce` binary for local execution.
