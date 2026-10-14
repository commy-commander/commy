# Site screenshots

`shoot.mjs` captures the screenshots in `public/screenshots/` from the desktop UI running on its
browser preview mock, with the demo community (Pixel Pioneers) and the art drawn in `art.mjs`:
character avatars in the mascot's family, a pixel-art farming game for the screen share and a
pixel-art castle shared in chat. The chat also carries a real GIF from KLIPY (a cute cartoon cat
saying hi); set `GIF_URL` to use another, or `GIF_URL=` for none. `og.mjs` renders `public/og.png`
from `og.html`.

1. Serve `apps/desktop/ui` on port 5174 (the `ui-preview` launch config does this, or
   `python3 -m http.server 5174` from that folder).
2. From this folder, with Playwright and Chromium available:

```bash
npm i --no-save playwright@1.48.2 && npx playwright install chromium
UI_URL=http://127.0.0.1:5174/ OUT=./out node shoot.mjs
```

Or in the Playwright image, which has Chromium:

```bash
docker run --rm --add-host=host.docker.internal:host-gateway -v "$PWD:/work" -w /work mcr.microsoft.com/playwright:v1.48.2-jammy sh -c "npm i --no-save playwright@1.48.2 && mkdir -p out && node shoot.mjs"
```

Pass scene names (`chat voice stream call roles invite signin`) to capture only some. Output lands in
`out/` (or `OUT`); copy the JPEGs to `public/screenshots/`. Set `NO_SANDBOX=1` when Chromium runs as
root. To show real cover art next to "Playing …", pass IGDB cover image ids by game name, for example
`COVERS='{"Minecraft":"co49x5"}'`; the app loads them from the public IGDB image CDN.

For the social image: `node og.mjs` writes `public/og.png`.
