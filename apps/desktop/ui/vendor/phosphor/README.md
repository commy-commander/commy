# Phosphor icons

The Studio look draws its icons from Phosphor. Each icon is kept in two weights, regular for an icon at rest and fill for an icon on a key or on the thing you are on, so icons load with the app and need no network request.

| File | Source | SHA-256 |
| --- | --- | --- |
| `icons.js` | npm `@phosphor-icons/core@2.1.1`, weights `regular` and `fill` | `737ac5573e932ea992f8a09702d77afb7cab34f97f14402afb77b4ec26425dc0` |

Licensed under the MIT License (`LICENSE`).

To rebuild, install `@phosphor-icons/core` in an empty folder and run `node apps/desktop/tools/studio-icons.mjs <that folder>/node_modules/@phosphor-icons/core/assets`. The icon names the app uses and the Phosphor drawing behind each are listed in `apps/desktop/tools/studio-icon-map.json`.
