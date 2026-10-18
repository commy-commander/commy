# Commy Stream Deck plugin

Plays Commy soundboard sounds from Stream Deck keys. It needs Stream Deck 7.1 or newer on Windows or macOS.

## Install

Download `commy-soundboard.streamDeckPlugin` from the latest Commy release and open it. In Commy, open **Settings > Integrations** and turn on **Stream Deck**. Drag **Play sound** from the Commy category onto a key, pick a server and a sound, and choose **Allow** when Commy asks.

## How it talks to Commy

Commy listens on `127.0.0.1` only, on port 47615 (test builds use 47616), and only while the Stream Deck switch is on. Each side sends one JSON object per line.

1. The plugin sends `{"type":"hello","version":1,"name":"Stream Deck","token":"..."}`.
2. A known token gets `{"type":"ready","version":1}`. Without one, Commy answers `{"type":"waiting"}` and asks the person in the app. Allow sends `{"type":"paired","token":"..."}` then `ready`; anything else sends `{"type":"denied","reason":"declined"}` and closes.
3. After `ready`, `{"type":"list"}` returns the soundboards and `{"type":"play","id":1,"server":"...","sound":"..."}` returns `played` or `failed` with a message. Leaving out `server` plays in the server you are in voice in.

The token lives in the plugin's global settings and in Commy's settings folder. Removing the Stream Deck in Commy revokes it, and the connection closes at its next request. Lines that are not this protocol, such as a web page posting to the port, close the connection without an answer.

## Develop

```sh
npm ci
npm test
npm run build
npm run validate
npm run pack
```

`npm run build` writes `chat.commy.soundboard.sdPlugin/bin/plugin.js`. Link the folder into Stream Deck with `npx streamdeck link chat.commy.soundboard.sdPlugin` to try it on a machine with Stream Deck installed.
