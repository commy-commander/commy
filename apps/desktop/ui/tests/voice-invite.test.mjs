import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { VOICE_INVITE_SLOTS, voiceInviteSummary, voiceInviteTitle, voiceInviteStatus, voiceInviteAction, voiceInviteIsFull } = require("../scripts/voice-invite.js");

const people = n => Array.from({ length: n }, (_, i) => ({ id: `u${i}`, name: `person${i}` }));

test("the participant summary shows at most six avatars and counts the rest", () => {
  assert.equal(VOICE_INVITE_SLOTS, 6);
  assert.deepEqual(voiceInviteSummary([]), { shown: [], overflow: 0, overflowLabel: "", count: 0, text: "Nobody is in the call yet" });
  const six = voiceInviteSummary(people(6));
  assert.equal(six.shown.length, 6);
  assert.equal(six.overflow, 0);
  assert.equal(six.overflowLabel, "");
  const nine = voiceInviteSummary(people(9));
  assert.equal(nine.shown.length, 6);
  assert.deepEqual(nine.shown.map(u => u.id), ["u0", "u1", "u2", "u3", "u4", "u5"]);
  assert.equal(nine.overflow, 3);
  assert.equal(nine.overflowLabel, "+3");
  assert.equal(nine.count, 9);
  assert.equal(voiceInviteSummary(people(40)).overflowLabel, "+34");
  assert.equal(voiceInviteSummary(undefined).count, 0);
  assert.equal(voiceInviteSummary(people(3), { slots: 2 }).overflow, 1);
});

test("the summary sentence names up to three people and counts beyond that", () => {
  assert.equal(voiceInviteSummary([{ id: "a", name: "ferris" }]).text, "ferris is in the call");
  assert.equal(voiceInviteSummary(["ferris", "nova"]).text, "ferris and nova are in the call");
  assert.equal(voiceInviteSummary(["ferris", "nova", "sleepy"]).text, "ferris, nova and sleepy are in the call");
  assert.equal(voiceInviteSummary(people(5)).text, "5 people are in the call");
  assert.equal(voiceInviteSummary([{ id: "x" }]).text, "Someone is in the call");
});

test("the title prefers live names over the ones stored in the card", () => {
  assert.equal(voiceInviteTitle({ channelName: "General", serverName: "Rust Hangout" }), "General · Rust Hangout");
  assert.equal(voiceInviteTitle({ channelName: "General", serverName: "Rust Hangout", live: { channel_name: "Lounge", server_name: "Rust Hangout" } }), "Lounge · Rust Hangout");
  assert.equal(voiceInviteTitle({ channelName: "", serverName: "" }), "a voice channel");
  assert.equal(voiceInviteTitle({ channelName: "General", serverName: "", live: null }), "General");
});

test("the status line says who is here, or that joining would reveal it", () => {
  assert.equal(voiceInviteStatus({ known: false }), "Join to see who is here");
  assert.equal(voiceInviteStatus({ known: true, gone: true }), "This voice channel is gone");
  assert.equal(voiceInviteStatus({ known: true, canView: false, users: people(2) }), "You cannot see this channel");
  assert.equal(voiceInviteStatus({ known: true, canView: true, users: [] }), "Nobody is in the call yet");
  assert.equal(voiceInviteStatus({ known: true, canView: true, users: ["ferris"], limit: 0 }), "ferris is in the call");
  assert.equal(voiceInviteStatus({ known: true, canView: true, users: ["ferris", "nova"], limit: 2 }), "ferris and nova are in the call · full");
});

test("join goes straight to the call when the viewer knows the server, otherwise through the invite link", () => {
  assert.deepEqual(voiceInviteAction({ known: true, canView: true, inThisCall: false, hasLink: false, full: false }), { label: "Join", kind: "join", enabled: true, note: "Join the call" });
  assert.deepEqual(voiceInviteAction({ known: true, inThisCall: true }), { label: "Joined", kind: "none", enabled: false, note: "You are in this call" });
  assert.deepEqual(voiceInviteAction({ known: false, hasLink: true }), { label: "Join", kind: "link", enabled: true, note: "Join the server, then the call" });
  assert.deepEqual(voiceInviteAction({ known: false, hasLink: false }), { label: "Join", kind: "none", enabled: false, note: "Ask for an invite to this server" });
  assert.equal(voiceInviteAction({ known: true, gone: true }).enabled, false);
  assert.equal(voiceInviteAction({ known: true, gone: true }).note, "This voice channel is gone");
  assert.equal(voiceInviteAction({ known: true, canView: false }).note, "You cannot see this channel");
  assert.deepEqual(voiceInviteAction({ known: true, canView: true, full: true }), { label: "Join", kind: "none", enabled: false, note: "This call is full" });
  assert.equal(voiceInviteAction({ known: true, canView: true, full: true, hasLink: true }).kind, "none", "a link does not get past a full channel");
});

test("a channel is full only against a limit and never for someone already inside", () => {
  assert.equal(voiceInviteIsFull({ users: people(3), limit: 0 }), false);
  assert.equal(voiceInviteIsFull({ users: people(3), limit: 3 }), true);
  assert.equal(voiceInviteIsFull({ users: people(2), limit: 3 }), false);
  assert.equal(voiceInviteIsFull({ users: people(3), limit: 3, inThisCall: true }), false);
  assert.equal(voiceInviteIsFull({ users: undefined, limit: 1 }), false);
});
