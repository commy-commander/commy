# Commy Stream Deck plugin

Controls Commy from Stream Deck keys and dials: plays and adds soundboard sounds, and handles voice. It needs Stream Deck 7.1 or newer on Windows or macOS.

## Install

Download `commy-soundboard.streamDeckPlugin` from the latest Commy release and open it. In Commy, open **Settings > Integrations** and turn on **Stream Deck**. Drag **Play sound** from the Commy category onto a key, pick a server and a sound, and choose **Allow** when Commy asks.

## Actions

| Action | What it does |
| --- | --- |
| Play sound | Plays a soundboard sound, and can add new sounds |
| Mute, Deafen | Toggle, and light up while on |
| Push to talk | Hold to talk while Commy is set to push to talk |
| Push to mute | Hold to mute for a moment |
| Voice input mode | Switches between voice activity and push to talk |
| Camera, Share screen | Toggle while you are in a voice channel; sharing opens Commy to pick a screen or game |
| Voice channel | Joins or leaves a chosen voice channel |
| Text channel | Opens a chosen channel in Commy |
| Volume, Person volume | Turn up, turn down or set; on a dial, turn to change |
| Audio device | Switches your microphone or speakers |
| Server stats | Shows online members of a server |
| Notifications | Shows your unread count and opens Commy |

Keys that show a state ask Commy for it about once a second, and only while they are on screen. If the plugin loses Commy while you hold Push to talk or Push to mute, Commy lets go of it.

To add a new sound, open **Add a new sound** in the key's settings, choose a server, pick an MP3, OGG or WAV file, move **Start** and **End** to keep up to 15 seconds, then give it a name and an optional emoji. Only servers where you can add sounds are listed: a role that can manage sounds, or a server with an open soundboard. After the upload the key plays the new sound.

## How it talks to Commy

Commy listens on `127.0.0.1` only, on port 47615 (test builds use 47616), and only while the Stream Deck switch is on. Each side sends one JSON object per line.

1. The plugin sends `{"type":"hello","version":1,"name":"Stream Deck","token":"..."}`.
2. A known token gets `{"type":"ready","version":1}`. Without one, Commy answers `{"type":"waiting"}` and asks the person in the app. Allow sends `{"type":"paired","token":"..."}` then `ready`; anything else sends `{"type":"denied","reason":"declined"}` and closes.
3. After `ready`, `{"type":"list"}` returns the soundboards and `{"type":"play","id":1,"server":"...","sound":"..."}` returns `played` or `failed` with a message. Leaving out `server` plays in the server you are in voice in.
4. `{"type":"add_sound","id":2,"server":"...","name":"...","emoji":"...","data":"<base64>"}` adds a sound with the same checks as adding one in the app, and returns `{"type":"added","id":2,"server":"...","sound":{...}}` or `failed`. Each server in the list says `can_add`. Commy never replaces an existing sound from here. Lines can be up to the size of a 1 MB sound after `ready`, and 4 KB before it.
5. `{"type":"status"}` returns `{"type":"status",...}` with mute, deafen, input mode, voice channel, camera, sharing, volumes, devices, person volumes, unread count and member counts per server. `{"type":"directory"}` returns servers with their text and voice channels, people you can set a volume for, and audio devices.
6. `{"type":"control","id":3,"control":{"action":"toggle_mute"}}` returns `{"type":"done","id":3,"status":{...}}` or `failed`. Actions: `toggle_mute`, `toggle_deafen`, `talk` and `hold_mute` with `down`, `toggle_input_mode`, `toggle_camera`, `toggle_stream`, `volume` with `target` (`output` or `input`) and `change` (`{"by":0.1}` or `{"to":1.0}`), `user_volume` with `user` and `change`, `device` with `target` and `device` (empty for the system default), `join_voice` and `open_channel` with `server` and `channel`, and `open_app`.

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
