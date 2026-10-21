import test from "node:test";
import assert from "node:assert/strict";
import { withCommyNeeds, PERMISSIONS, FEATURES, COMPONENTS } from "./android-project.mjs";

const generated = `<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android">
    <uses-permission android:name="android.permission.INTERNET" />

    <application
        android:label="@string/app_name">
    </application>
</manifest>
`;

test("adds every permission and feature once, before the application element", () => {
  const out = withCommyNeeds(generated);
  for (const name of [...PERMISSIONS, ...FEATURES]) {
    assert.equal(out.split(`"${name}"`).length - 1, 1, name);
    assert.ok(out.indexOf(`"${name}"`) < out.indexOf("<application"), name);
  }
});

test("running twice changes nothing", () => {
  const once = withCommyNeeds(generated);
  assert.equal(withCommyNeeds(once), once);
});

test("the hardware is optional so phones without a camera can still install", () => {
  const out = withCommyNeeds(generated);
  assert.match(out, /"android.hardware.camera" android:required="false"/);
});

test("the call service and relaunch screen are declared once, inside the application element", () => {
  const out = withCommyNeeds(withCommyNeeds(generated));
  for (const { name } of COMPONENTS) {
    assert.equal(out.split(`"${name}"`).length - 1, 1, name);
    assert.ok(out.indexOf(`"${name}"`) > out.indexOf("<application"), name);
    assert.ok(out.indexOf(`"${name}"`) < out.indexOf("</application>"), name);
  }
  assert.match(out, /android:exported="false" android:foregroundServiceType="microphone\|mediaPlayback"/);
  assert.match(out, /"\.RelaunchActivity" android:process=":relaunch"/);
});

test("a manifest without an application element is refused", () => {
  assert.throws(() => withCommyNeeds("<manifest></manifest>"));
});
