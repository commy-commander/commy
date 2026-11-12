# Contributing to Commy

Thanks for helping build Commy. This file covers the branch flow and the
checks a change needs before it lands. Everyone taking part is expected to
follow the [Code of Conduct](CODE_OF_CONDUCT.md). To report a security problem,
see [SECURITY.md](SECURITY.md).

## Branch flow

- `main` is protected and always releasable. Nobody pushes to it directly.
- Do all work on a branch cut from the latest `main`, named after the
  change's Conventional Commits type:
  - `feat/<short-name>` for new features
  - `fix/<short-name>` for fixes
  - `docs/<short-name>` for documentation
  - `ci/<short-name>`, `chore/<short-name>` and so on for the other types
- Open a pull request into `main`. It merges once checks pass and it has
  been reviewed.
- Delete the branch after it merges.

```sh
git switch main
git pull
git switch -c feat/my-change
# ...work, commit...
git push -u origin feat/my-change
```

## Commits

Use [Conventional Commits](https://www.conventionalcommits.org):
`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`, with an optional
scope such as `feat(ui):` or `fix(net):`.

## Code style

No code comments. Say it in the code instead: clear names, named constants,
small functions, and tests for the behaviour that matters. Rust doc comments
count as comments. The only exceptions are license or SPDX headers,
`// SAFETY:` notes on unsafe blocks, and tool directives such as `#[allow]`,
`eslint-disable` or `# shellcheck disable`.

## Before opening a PR

```sh
cargo clippy --workspace
cargo test --workspace
```

New modules need unit tests. UI changes should be checked in the running
app (`cargo tauri dev`) since the frontend is embedded at build time.

## Never commit

- `.env` files, API keys, OAuth secrets, or anything from your app data
  directory (`integrations.json` holds IGDB credentials)
- `target/`, `node_modules/`, and other build output

## License

By contributing you agree that your contributions are licensed under the
GNU Affero General Public License v3.0, the same license as the project.
