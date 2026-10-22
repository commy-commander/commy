# Device tests

Runs Commy's phone flows end to end on a real Android device over adb, with a desktop build as the other person. It reads the device's log, checks notifications, and calls the apps' own commands through their webviews' debug ports. It never taps the screen.

## What it checks

| Scenario | Passes when |
| --- | --- |
| `launch` | Commy installs (when an APK is given), starts, connects its webview and has not crashed 10 seconds later |
| `accounts` | The device and the desktop are signed in to the same account server as two different people, and the device is linked |
| `wake` | The device registers for wake-ups, is closed the way a swipe-away closes it, and a DM from the desktop wakes it with a notification. The report has the latency |
| `burst` | Three DMs 5 seconds apart, sent while the device is closed, wake it once right away and once more when the push gap ends |
| `badge` | Opening Commy clears its notifications |
| `delivery` | Every DM the run sent stops showing as waiting to send |

## Before a run

- An Android device with USB debugging allowed for this computer, and a debug build of Commy on it (`--apk` or `--run` installs one). Release builds have no webview debug port.
- The device and a desktop data folder signed in to the same account server as two different people who share a server or have a DM. Sign-in goes through the browser, so it is not automated.
- Windows for the desktop side. The run starts the desktop build with WebView2's `--remote-debugging-port`. On other systems, start a build yourself and set `bench.attach`.
- Node 22 or newer, `adb`, and the GitHub CLI for `--run`.

## Config

Keep it outside the repository; it names your own folders.

```json
{
  "adb": "path/to/adb",
  "serial": "device serial from adb devices",
  "repo": "commy-commander/commy",
  "pushGapSeconds": 15,
  "bench": {
    "exe": "path/to/commy-desktop.exe",
    "dataDir": "path/to/a/signed-in/data/folder",
    "authUrl": "https://dev.commy.chat",
    "debugPort": 9334
  }
}
```

`pushGapSeconds` must match the sync service's `PUSH_GAP_SECS`. The desktop build must not already be running on that data folder.

## Running

```
node apps/desktop/mobile/e2e/run.mjs --config e2e.json
node apps/desktop/mobile/e2e/run.mjs --config e2e.json --run <Mobile builds run id> --only launch,accounts
node apps/desktop/mobile/e2e/run.mjs --config e2e.json --only wake --report report.json
```

It prints one line per scenario and exits with 1 if any failed (2 if it could not start). `--report` writes the details as JSON. The desktop side's log goes to `e2e-log.txt` in its data folder.

Installing a CI build whose signing key differs from the installed one uninstalls the app first, which signs the device out.
