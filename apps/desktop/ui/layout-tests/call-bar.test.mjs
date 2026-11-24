import test from "node:test";
import assert from "node:assert/strict";
import { chromium } from "playwright";
import { serveUi } from "./serve.mjs";

const WIDTHS = [940, 1000, 1040, 1041, 1100, 1179, 1180, 1240, 1241, 1339, 1366, 1440, 1600, 1920, 2560, 3440];
const HEIGHTS = [640, 759, 900, 1440];
const LANGUAGES = ["en", "es"];
const STATES = [
  { name: "chat open", folded: true, chat: true },
  { name: "chat closed", folded: true, chat: false },
  { name: "channels and chat open", folded: false, chat: true },
  { name: "channels open, chat closed", folded: false, chat: false },
  { name: "chat dragged wide", folded: true, chat: true, chatWidth: 640 },
  { name: "channels open, chat dragged wide", folded: false, chat: true, chatWidth: 640 },
  { name: "sharing, channels open, chat dragged wide", folded: false, chat: true, chatWidth: 640, sharing: true },
];
const SLACK = 0.5;

async function joinTheVoiceRoom(page) {
  await page.waitForFunction(() => typeof state !== "undefined" && Array.isArray(state.servers) && state.servers.length > 0);
  await page.evaluate(async () => {
    const busy = Object.values(mock.servers).find(server => (server.channels || []).some(channel => (channel.voice_user_ids || []).length > 3));
    busy.needs_rules = false;
    await selectServer(busy.id);
    const voice = state.currentServer.channels.find(channel => channel.kind === "voice");
    await selectChannel(voice);
    await toggleVoice(voice);
    await refreshCurrentServer();
    await showCallGrid();
  });
}

async function wearLook(page, look) {
  await page.evaluate(look => {
    voiceRoom.folded = look.folded;
    voiceRoom.chat = look.chat;
    paintVoiceRoomChatWidth(look.chatWidth || null);
    state.sharing = Boolean(look.sharing);
    renderShareButton();
  }, look);
}

async function measureCallBar(page) {
  return page.evaluate(() => {
    placeVoiceRoom();
    const shown = node => node.getClientRects().length > 0 && getComputedStyle(node).visibility !== "hidden";
    const box = node => {
      const rect = node.getBoundingClientRect();
      return { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom };
    };
    const bar = document.getElementById("voice-panel");
    const parts = [...bar.querySelectorAll(".vp-conn-meta, .vp-voice .split-btn, .vp-voice > .icon-btn, .vp-actions > button, .vp-meter, #vp-disconnect")]
      .filter(shown)
      .map(node => ({ name: node.id || node.className.split(" ")[0], ...box(node) }))
      .filter(part => part.right > part.left);
    return { open: document.getElementById("app").classList.contains("vr-open"), bar: box(bar), parts };
  });
}

function outside(part, bar) {
  return part.left < bar.left - SLACK || part.right > bar.right + SLACK || part.top < bar.top - SLACK || part.bottom > bar.bottom + SLACK;
}

function overlapping(a, b) {
  return a.left < b.right - SLACK && b.left < a.right - SLACK && a.top < b.bottom - SLACK && b.top < a.bottom - SLACK;
}

function callBarProblems({ open, bar, parts }) {
  const problems = [];
  if (!open) problems.push("the voice room is not open");
  if (!parts.some(part => part.name === "vp-disconnect")) problems.push("there is no end call key");
  for (const part of parts) if (outside(part, bar)) problems.push(`${part.name} sticks out of the bar`);
  parts.forEach((a, i) => parts.slice(i + 1).forEach(b => {
    if (overlapping(a, b)) problems.push(`${a.name} overlaps ${b.name}`);
  }));
  return problems;
}

async function sweepLook(browser, url, language, look) {
  const page = await browser.newPage({ viewport: { width: WIDTHS[0], height: HEIGHTS[0] } });
  await page.addInitScript(code => localStorage.setItem("commy.accessibility", JSON.stringify({ language: code })), language);
  await page.goto(`${url}/index.html?studio&streams`);
  await joinTheVoiceRoom(page);
  await wearLook(page, look);
  const failures = [];
  for (const width of WIDTHS) {
    for (const height of HEIGHTS) {
      await page.setViewportSize({ width, height });
      const problems = callBarProblems(await measureCallBar(page));
      if (problems.length) failures.push(`${language} ${width}x${height} ${look.name}: ${problems.join(", ")}`);
    }
  }
  await page.close();
  return failures;
}

test("every call bar key, End included, stays inside the bar at every window size", async () => {
  const ui = await serveUi();
  const browser = await chromium.launch();
  try {
    const sweeps = LANGUAGES.flatMap(language => STATES.map(look => sweepLook(browser, ui.url, language, look)));
    const failures = (await Promise.all(sweeps)).flat();
    assert.deepEqual(failures, []);
  } finally {
    await browser.close();
    await ui.close();
  }
});

test("the sweep notices an End key pushed out of the bar", () => {
  const bar = { left: 0, right: 600, top: 0, bottom: 66 };
  const end = { name: "vp-disconnect", left: 580, right: 626, top: 10, bottom: 56 };
  assert.deepEqual(callBarProblems({ open: true, bar, parts: [end] }), ["vp-disconnect sticks out of the bar"]);
});

test("the sweep notices keys that crowd each other", () => {
  const bar = { left: 0, right: 600, top: 0, bottom: 66 };
  const sound = { name: "vp-soundboard", left: 400, right: 540, top: 13, bottom: 53 };
  const meter = { name: "vp-meter", left: 500, right: 556, top: 21, bottom: 45 };
  const end = { name: "vp-disconnect", left: 544, right: 590, top: 10, bottom: 56 };
  assert.deepEqual(callBarProblems({ open: true, bar, parts: [sound, meter, end] }), ["vp-soundboard overlaps vp-meter", "vp-meter overlaps vp-disconnect"]);
});
