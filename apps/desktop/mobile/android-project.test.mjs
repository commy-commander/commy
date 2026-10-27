import test from "node:test";
import assert from "node:assert/strict";
import { withCommyNeeds, withCommyDependencies, withDebugSigning, DEBUG_KEY, applicationId, firebaseValues, firebasePackages, readGoogleServices, PERMISSIONS, FEATURES, COMPONENTS, GRADLE_DEPENDENCIES, INVITE_LINKS, MAX_SDK } from "./android-project.mjs";

const generated = `<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android">
    <uses-permission android:name="android.permission.INTERNET" />

    <application
        android:label="@string/app_name">
        <activity
            android:launchMode="singleTask"
            android:name=".MainActivity"
            android:exported="true">
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity>
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
  assert.match(out, /"\.ScreenShareService" android:exported="false" android:foregroundServiceType="mediaProjection"/);
});

test("a manifest without an application element is refused", () => {
  assert.throws(() => withCommyNeeds("<manifest></manifest>"));
});

const gradle = `plugins {
    id("com.android.application")
}

android {
    defaultConfig {
        applicationId = "chat.commy.mobile"
    }
}

dependencies {
    implementation("androidx.webkit:webkit:1.14.0")
}
`;

test("the push library is added to the app's dependencies once", () => {
  const out = withCommyDependencies(withCommyDependencies(gradle));
  for (const coordinate of GRADLE_DEPENDENCIES) {
    assert.equal(out.split(`implementation("${coordinate}")`).length - 1, 1, coordinate);
    assert.ok(out.indexOf(coordinate) > out.indexOf("dependencies {"), coordinate);
  }
  assert.ok(out.includes(`implementation("androidx.webkit:webkit:1.14.0")`));
});

test("a build file without a dependencies block is refused", () => {
  assert.throws(() => withCommyDependencies("android {}\n"));
});

test("the application id is read from the build file", () => {
  assert.equal(applicationId(gradle), "chat.commy.mobile");
  assert.equal(applicationId("android {}"), null);
});

const googleServices = {
  project_info: { project_number: "123456", project_id: "commy-test" },
  client: [
    { client_info: { mobilesdk_app_id: "1:123456:android:other", android_client_info: { package_name: "other.app" } }, api_key: [{ current_key: "other-key" }] },
    { client_info: { mobilesdk_app_id: "1:123456:android:abc", android_client_info: { package_name: "chat.commy.mobile" } }, api_key: [{ current_key: "AIza<&>\\'" }] },
  ],
};

test("Firebase settings come from the client for this package and are kept by the shrinker", () => {
  const xml = firebaseValues(JSON.stringify(googleServices), "chat.commy.mobile");
  assert.match(xml, /<string name="google_app_id" translatable="false">1:123456:android:abc<\/string>/);
  assert.match(xml, /<string name="gcm_defaultSenderId" translatable="false">123456<\/string>/);
  assert.match(xml, /<string name="project_id" translatable="false">commy-test<\/string>/);
  assert.ok(xml.includes("AIza&lt;&amp;&gt;\\\\\\'</string>"));
  assert.ok(!xml.includes("other-key"));
  assert.match(xml, /tools:keep="@string\/google_app_id,/);
});

test("no Firebase settings when the file has no app for this package or misses a value", () => {
  assert.equal(firebaseValues(googleServices, "chat.commy.elsewhere"), null);
  const noKey = structuredClone(googleServices);
  noKey.client[1].api_key = [];
  assert.equal(firebaseValues(noKey, "chat.commy.mobile"), null);
});

test("the wake-up receiver listens for Firebase messages without being exported", () => {
  const out = withCommyNeeds(generated);
  assert.match(out, /"\.WakeService" android:exported="false"><intent-filter><action android:name="com.google.firebase.MESSAGING_EVENT" \/>/);
});

test("GOOGLE_SERVICES_JSON may be the file itself, its base64 or a path", () => {
  const json = JSON.stringify(googleServices);
  const noFile = () => assert.fail("no file is read");
  assert.equal(readGoogleServices(json, noFile), json);
  assert.equal(readGoogleServices(Buffer.from(json).toString("base64"), noFile), json);
  assert.equal(readGoogleServices("C:\\keys\\google-services.json", path => `read ${path}`), "read C:\\keys\\google-services.json");
  assert.equal(readGoogleServices("  ", noFile), null);
  assert.equal(readGoogleServices(undefined, noFile), null);
});

test("the Android packages in the Firebase file can be listed to explain a mismatch", () => {
  assert.deepEqual(firebasePackages(JSON.stringify(googleServices)), ["other.app", "chat.commy.mobile"]);
  assert.deepEqual(firebasePackages({}), []);
});

test("invite links open the main screen, once, through the commy scheme", () => {
  const out = withCommyNeeds(withCommyNeeds(generated));
  assert.equal(out.split(INVITE_LINKS).length - 1, 1);
  const main = out.indexOf('".MainActivity"');
  const filter = out.indexOf(INVITE_LINKS);
  assert.ok(filter > main && filter < out.indexOf("</activity>", main));
  assert.match(INVITE_LINKS, /android.intent.category.BROWSABLE/);
  assert.match(INVITE_LINKS, /android:scheme="commy" android:host="i"/);
});

test("a manifest without the main screen is refused", () => {
  assert.throws(() => withCommyNeeds(generated.replace('android:name=".MainActivity"', 'android:name=".Other"')), /MainActivity/);
});

test("the old Bluetooth permission only applies where Android still needs it for headsets", () => {
  const out = withCommyNeeds(generated);
  assert.match(out, /"android.permission.BLUETOOTH" android:maxSdkVersion="30"/);
  assert.equal(MAX_SDK["android.permission.BLUETOOTH"], 30);
});

const tauriGradle = `android {
    namespace = "chat.commy.mobile"
    buildTypes {
        getByName("debug") {
            isDebuggable = true
        }
    }
}
`;

test("debug builds sign with the kept key when one is given, once", () => {
  const out = withDebugSigning(withDebugSigning(tauriGradle, "/home/runner/.android/debug.keystore"), "/home/runner/.android/debug.keystore");
  assert.equal(out.split("signingConfigs {").length - 1, 1);
  assert.ok(out.indexOf("signingConfigs {") < out.indexOf("buildTypes {"));
  assert.match(out, /^    signingConfigs \{\n        getByName\("debug"\) \{\n            storeFile = file\("\/home\/runner\/\.android\/debug\.keystore"\)$/m);
  assert.ok(out.includes(`keyAlias = "${DEBUG_KEY.keyAlias}"`));
  assert.ok(out.includes(`storePassword = "${DEBUG_KEY.storePassword}"`));
  assert.ok(out.includes(`keyPassword = "${DEBUG_KEY.keyPassword}"`));
});

test("without a kept key the build file is left alone", () => {
  assert.equal(withDebugSigning(tauriGradle, undefined), tauriGradle);
  assert.equal(withDebugSigning(tauriGradle, ""), tauriGradle);
});

test("a key path is written as a Kotlin string that cannot break out", () => {
  const out = withDebugSigning(tauriGradle, 'C:\\keys\\"$x".keystore');
  assert.ok(out.includes('storeFile = file("C:\\\\keys\\\\\\"\\$x\\".keystore")'));
});

test("a build file without build types is refused when a key is given", () => {
  assert.throws(() => withDebugSigning("android {\n}\n", "/k"));
});
