# Site screenshots

`shoot.mjs` captures the screenshots in `public/screenshots/` from the desktop UI running on its
browser preview mock, with the demo community and art from `art.mjs`. `og.mjs` renders `public/og.png`
from `og.html`.

1. Serve `apps/desktop/ui` on port 5174 (the `ui-preview` launch config does this).
2. From this folder, run in the Playwright image (it has Chromium):

```bash
docker run --rm --add-host=host.docker.internal:host-gateway -v "$PWD:/work" -w /work mcr.microsoft.com/playwright:v1.48.2-jammy sh -c "npm i --no-save playwright@1.48.2 && mkdir -p out && node shoot.mjs"
```

Pass scene names (`chat voice stream call roles invite signin`) to capture only some. Output lands in
`out/`; copy the JPEGs to `public/screenshots/`. For `og.mjs`, copy `public/logo.png`, `out/chat.jpg`
and the Inter 500/800 fonts from `app/fonts/` into `out/` first.
