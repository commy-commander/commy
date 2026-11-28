# Fluent icons

The app draws its icons from Microsoft's Fluent UI System Icons. Each icon is kept at 24 px in two weights, regular for an icon at rest and filled for an icon on a key, a bar or a tab, so icons load with the app and need no network request.

| File | Source | SHA-256 |
| --- | --- | --- |
| `icons.js` | npm `@fluentui/svg-icons@1.1.300`, 24 px, weights `regular` and `filled` | `ce25df7fcb4e951b46896e1349470f74d56eb5ed8582c9e76e58df550033b4f6` |

Licensed under the MIT License (`LICENSE`).

To rebuild, install `@fluentui/svg-icons` in an empty folder and run `node apps/desktop/tools/studio-icons.mjs <that folder>/node_modules/@fluentui/svg-icons/icons`. The icon names the app uses and the Fluent drawing behind each are listed in `apps/desktop/tools/studio-icon-map.json`.
