import test from "node:test";
import assert from "node:assert/strict";
import { generateKeyPairSync, createVerify } from "node:crypto";
import { readServiceAccount, signedAssertion, versionCodeOf, trackRelease, uploadBundle, PACKAGE } from "./play-upload.mjs";

const { privateKey, publicKey } = generateKeyPairSync("rsa", { modulusLength: 2048 });
const keyJson = JSON.stringify({ type: "service_account", client_email: "play-upload@commy.iam.gserviceaccount.com", private_key: privateKey.export({ type: "pkcs8", format: "pem" }), token_uri: "https://oauth2.example/token" });

test("the service account key must carry an email and a private key", () => {
  assert.throws(() => readServiceAccount(""), /not valid JSON/);
  assert.throws(() => readServiceAccount("{}"), /no client_email/);
  assert.equal(readServiceAccount(keyJson).email, "play-upload@commy.iam.gserviceaccount.com");
  assert.equal(readServiceAccount(JSON.stringify({ client_email: "a", private_key: "b" })).tokenUri, "https://oauth2.googleapis.com/token");
});

test("the sign-in assertion is a JWT signed by the service account for the Play publishing scope", () => {
  const account = readServiceAccount(keyJson);
  const [header, claims, signature] = signedAssertion(account, 1000).split(".");
  const verify = createVerify("RSA-SHA256");
  verify.update(`${header}.${claims}`);
  assert.ok(verify.verify(publicKey, Buffer.from(signature, "base64url")));
  const body = JSON.parse(Buffer.from(claims, "base64url").toString());
  assert.deepEqual(body, { iss: account.email, scope: "https://www.googleapis.com/auth/androidpublisher", aud: "https://oauth2.example/token", iat: 1000, exp: 4600 });
});

test("the version code comes from the bundle name the build gives it", () => {
  assert.equal(versionCodeOf("dist/Commy-0.1.17-445952.aab"), "445952");
  assert.equal(versionCodeOf("Commy-0.1.17-rc.1-12.aab"), "12");
  assert.throws(() => versionCodeOf("app-release.aab"), /version code/);
});

test("release notes are capped at Play's 500 characters", () => {
  const body = trackRelease({ track: "internal", name: "0.1.17", versionCode: 7, status: "completed", notes: "x".repeat(600) });
  assert.deepEqual(body.releases[0].versionCodes, ["7"]);
  assert.equal(body.releases[0].releaseNotes[0].text.length, 500);
  assert.equal(trackRelease({ track: "internal", name: "n", versionCode: 1, status: "draft" }).releases[0].releaseNotes, undefined);
});

function fakePlay({ draftApp = false } = {}) {
  const calls = [];
  const fetch = async (url, init) => {
    calls.push({ url, method: init.method, body: init.body });
    const reply = (status, body) => ({ ok: status < 300, status, text: async () => JSON.stringify(body) });
    if (url.endsWith("/token")) return reply(200, { access_token: "tok" });
    if (url.endsWith("/edits")) return reply(200, { id: "e1" });
    if (url.includes("/bundles")) return reply(200, { versionCode: 445952 });
    if (url.includes("/tracks/")) {
      const status = JSON.parse(init.body).releases[0].status;
      if (draftApp && status !== "draft") return reply(400, { error: { message: "Only releases with status draft may be created on draft app." } });
      return reply(200, {});
    }
    if (url.endsWith(":commit")) return reply(200, {});
    return reply(404, { error: { message: "unexpected" } });
  };
  return { calls, fetch };
}

test("a bundle is uploaded into one edit, put on the chosen testing track and committed", async () => {
  const play = fakePlay();
  const result = await uploadBundle({ account: readServiceAccount(keyJson), bundle: Buffer.from("aab"), versionCode: "445952", track: "closed", name: "0.1.17", fetch: play.fetch, log: () => {} });
  assert.deepEqual(result, { versionCode: "445952", track: "closed", status: "completed" });
  assert.deepEqual(play.calls.map(c => `${c.method} ${c.url.replace(/^https:\/\/[^/]+/, "")}`), [
    "POST /token",
    `POST /androidpublisher/v3/applications/${PACKAGE}/edits`,
    `POST /upload/androidpublisher/v3/applications/${PACKAGE}/edits/e1/bundles?uploadType=media`,
    `PUT /androidpublisher/v3/applications/${PACKAGE}/edits/e1/tracks/alpha`,
    `POST /androidpublisher/v3/applications/${PACKAGE}/edits/e1:commit`,
  ]);
});

test("while Play still calls the app a draft, the release is saved as a draft instead of failing", async () => {
  const play = fakePlay({ draftApp: true });
  const result = await uploadBundle({ account: readServiceAccount(keyJson), bundle: Buffer.from("aab"), versionCode: "1", name: "n", fetch: play.fetch, log: () => {} });
  assert.equal(result.status, "draft");
  assert.ok(play.calls.at(-1).url.endsWith(":commit"));
});

test("production is never a choice and errors say what Play answered", async () => {
  await assert.rejects(uploadBundle({ account: readServiceAccount(keyJson), bundle: Buffer.from(""), versionCode: "1", track: "production", name: "n", fetch: fakePlay().fetch }), /internal, closed, open/);
  const denied = async () => ({ ok: false, status: 403, text: async () => JSON.stringify({ error: { message: "The caller does not have permission" } }) });
  await assert.rejects(uploadBundle({ account: readServiceAccount(keyJson), bundle: Buffer.from(""), versionCode: "1", name: "n", fetch: denied }), /Signing in to Google failed \(403\): The caller does not have permission/);
});
