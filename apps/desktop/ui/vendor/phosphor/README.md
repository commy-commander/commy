# Phosphor icons

The Studio look draws its icons from Phosphor. Each icon is kept in two weights, regular for an icon at rest and fill for an icon on a key or on the thing you are on, so icons load with the app and need no network request.

| File | Source | SHA-256 |
| --- | --- | --- |
| `icons.js` | npm `@phosphor-icons/core@2.1.1`, weights `regular` and `fill` | `a2e86833f83891d39a4cd4d7f105519b2b84384d63b57eac2c5596aa038e6706` |

Licensed under the MIT License (`LICENSE`).

To rebuild, install `@phosphor-icons/core` in an empty folder and run `node apps/desktop/tools/studio-icons.mjs <that folder>/node_modules/@phosphor-icons/core/assets`. The icon names the app uses and the Phosphor drawing behind each are listed in `apps/desktop/tools/studio-icon-map.json`.
