# Verifying a download

Every release ships `SHA256SUMS.txt` with the hash of each file, signed by the
release workflow with [cosign](https://github.com/sigstore/cosign).

Releases built on git.commy.chat sign it with the Commy release key,
[`deploy/agent/cosign.pub`](../deploy/agent/cosign.pub), the same key that signs
the server images. With the file, `SHA256SUMS.txt`, `SHA256SUMS.txt.bundle` and
`cosign.pub` in one folder:

```bash
cosign verify-blob --key cosign.pub --bundle SHA256SUMS.txt.bundle SHA256SUMS.txt
sha256sum --check --ignore-missing SHA256SUMS.txt
```

The first command proves the checksum list was signed with the release key, the
second that your file matches it.

Releases built on GitHub were signed keyless, through GitHub's OIDC identity,
and every file carries a GitHub build provenance attestation. With the file,
`SHA256SUMS.txt`, `SHA256SUMS.txt.sig` and `SHA256SUMS.txt.pem` in one folder:

```bash
cosign verify-blob --certificate SHA256SUMS.txt.pem --signature SHA256SUMS.txt.sig \
  --certificate-identity-regexp '^https://github.com/commy-commander/commy/\.github/workflows/release\.yml@refs/tags/v' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com SHA256SUMS.txt
sha256sum --check --ignore-missing SHA256SUMS.txt
gh attestation verify Commy_<version>_x64-setup.exe --repo commy-commander/commy
```

The first command proves the checksum list was produced by this repository's
release workflow on a release tag, the second that your file matches it, and
the third (with the [GitHub CLI](https://cli.github.com)) that the file itself
was built by that workflow from this repository.

Builds are not code signed yet, so Windows SmartScreen and macOS Gatekeeper
will ask you to confirm before the first launch.
