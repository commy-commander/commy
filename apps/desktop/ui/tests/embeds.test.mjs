import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { MAX_IMAGE_EMBEDS, MAX_VIDEO_LINK_EMBEDS, youtubeThumbnail, videoLinkEmbeds, MAX_AUTO_IMAGE_BYTES, isMediaHost, relaysImage, imageHost, imageLinks, embedMode, imageGateText, loadsImageOnItsOwn, largeImageGateText } = require("../scripts/embeds.js");

const KLIPY = "https://static.klipy.com/ii/4e7b/fd/3b/5wGhb3j5.gif";
const GIPHY = "https://media2.giphy.com/media/v1.Y2lkPTc5MGI3NjExYWJj/giphy.gif?cid=790b7611abc&rid=giphy.gif&ct=g";
const OTHER = "https://pics.example.org/cat.png";

test("only the hosts the account server proxies count as media hosts", () => {
  for (const host of ["static.klipy.com", "images.igdb.com", "media.giphy.com", "media0.giphy.com", "media4.giphy.com", "media9.giphy.com", "i.giphy.com", "I.GIPHY.COM", "cdn3.emoji.gg", "media.tenor.com", "media1.tenor.com", "c.tenor.com", "i.ytimg.com", "i9.ytimg.com", "img.youtube.com"]) {
    assert.ok(isMediaHost(host), host);
  }
  for (const host of ["giphy.com", "api.giphy.com", "media10.giphy.com", "media0.giphy.com.evil.example", "evilmedia0.giphy.com", "static.klipy.com.evil.example", "klipy.com", "emoji.gg", "cdn3.emoji.gg.evil.example", "tenor.com", "media.tenor.com.evil.example", "youtube.com", "www.youtube.com", "img.youtube.com.evil.example", "example.org", "", null]) {
    assert.equal(isMediaHost(host), false, String(host));
  }
});

test("the host of an image link is read from the URL, https only", () => {
  assert.equal(imageHost(OTHER), "pics.example.org");
  assert.equal(imageHost("https://static.klipy.com@evil.example/a.gif"), "evil.example");
  assert.equal(imageHost("http://pics.example.org/cat.png"), null);
  assert.equal(imageHost("not a url"), null);
  assert.equal(imageHost(""), null);
  assert.equal(imageHost(undefined), null);
});

test("image links are https links with an image extension, at most four", () => {
  assert.deepEqual(imageLinks(`look ${OTHER} and ${GIPHY}`), [OTHER, GIPHY]);
  assert.deepEqual(imageLinks("https://a.example/x.webp?size=2 https://b.example/y.jpeg"), ["https://a.example/x.webp?size=2", "https://b.example/y.jpeg"]);
  assert.deepEqual(imageLinks("http://a.example/x.gif"), [], "plain http is never embedded");
  assert.deepEqual(imageLinks("https://a.example/photo"), [], "a lone link without an extension is never embedded");
  assert.deepEqual(imageLinks("https://a.example/page.html"), []);
  assert.deepEqual(imageLinks("https://a.example/page?next=x.gif"), [], "the extension has to end the path");
  assert.deepEqual(imageLinks(""), []);
  assert.deepEqual(imageLinks(null), []);
  const many = Array.from({ length: 6 }, (_, i) => `https://a.example/${i}.png`);
  assert.equal(imageLinks(many.join(" ")).length, MAX_IMAGE_EMBEDS);
});

test("shared GIPHY and Tenor links go through the account server", () => {
  for (const url of [
    "https://media.giphy.com/media/v1.Y2lkPTc5MGI3NjExNHB4eHQ3b2F4cHd4/l0MYt5jPR6QX5pnqM/giphy.gif",
    "https://media1.giphy.com/media/v1.Y2lkPTc5MGI3NjEx/3o7TKSjRrfIPjeiVyM/giphy.gif?cid=790b7611&ep=v1_gifs&rid=giphy.gif&ct=g",
    "https://media.giphy.com/media/l0MYt5jPR6QX5pnqM/giphy.gif",
    "https://i.giphy.com/l0MYt5jPR6QX5pnqM.webp",
    "https://media.tenor.com/x8v1oNUOmg4AAAAC/rickroll-roll.gif",
    "https://media1.tenor.com/m/x8v1oNUOmg4AAAAC/rickroll-roll.gif",
    "https://c.tenor.com/x8v1oNUOmg4AAAAC/tenor.gif",
  ]) {
    assert.ok(relaysImage(url), url);
    assert.equal(embedMode(url), "auto", url);
  }
});

test("links on a media host that the account server would refuse wait for a click", () => {
  for (const url of [
    "https://media1.giphy.com/media/a/b/c/giphy.gif",
    "https://media1.giphy.com/media/3o7aD2saalBwwftBIY/giphy.gif?" + "x".repeat(300),
    "https://media.tenor.com/x8v1oNUOmg4AAAAC/rickroll-roll.gif?size=2",
    "https://cdn3.emoji.gg/stickers/790374-smile.png",
    "https://images.igdb.com/other/path.jpg",
    "https://static.klipy.com:8443/a.gif",
  ]) {
    assert.equal(relaysImage(url), false, url);
    assert.equal(embedMode(url), "click", url);
  }
  assert.equal(relaysImage("not a url"), false);
  assert.equal(relaysImage(OTHER), false);
});

test("YouTube thumbnails go through the account server", () => {
  for (const url of [
    "https://i.ytimg.com/vi/Sp0ZZuNrXh8/sddefault.jpg",
    "https://i.ytimg.com/vi/Sp0ZZuNrXh8/maxresdefault.jpg",
    "https://i.ytimg.com/vi_webp/Sp0ZZuNrXh8/hqdefault.webp",
    "https://i9.ytimg.com/vi/Sp0ZZuNrXh8/hq720.jpg?sqp=-oaymwEcCNAFEJQDSFXyq4qpAw4IARUAAIhCGAFwAcABBg&rs=AOn4CLA",
    "https://img.youtube.com/vi/Sp0ZZuNrXh8/hqdefault.jpg",
    "https://img.youtube.com/vi_webp/Sp0ZZuNrXh8/hqdefault.webp",
  ]) {
    assert.ok(relaysImage(url), url);
    assert.equal(embedMode(url), "auto", url);
    assert.deepEqual(imageLinks(`look ${url}`), [url], url);
  }
  for (const url of [
    "https://www.youtube.com/vi/Sp0ZZuNrXh8/hqdefault.jpg",
    "https://i.ytimg.com/vi/short/sddefault.jpg",
    "https://i.ytimg.com/an/Sp0ZZuNrXh8/sddefault.jpg",
    "https://i.ytimg.com/vi/Sp0ZZuNrXh8/sddefault.png",
    "https://i10.ytimg.com/vi/Sp0ZZuNrXh8/sddefault.jpg",
    "https://i.ytimg.com.evil.example/vi/Sp0ZZuNrXh8/sddefault.jpg",
  ]) {
    assert.equal(relaysImage(url), false, url);
  }
});

test("proxied hosts embed on their own, everything else waits for a click", () => {
  assert.equal(embedMode(KLIPY), "auto");
  assert.equal(embedMode(GIPHY), "auto");
  assert.equal(embedMode("https://images.igdb.com/igdb/image/upload/t_thumb/co1rs4.jpg"), "auto");
  assert.equal(embedMode(OTHER), "click");
  assert.equal(embedMode("https://static.klipy.com@evil.example/a.gif"), "click");
  assert.equal(embedMode(KLIPY, false), "click", "without the account server route nothing loads on its own");
  assert.equal(embedMode("http://static.klipy.com/a.gif"), "none");
  assert.equal(embedMode("nope"), "none");
});

test("the placeholder names the host in every line it shows", () => {
  const text = imageGateText("pics.example.org");
  assert.equal(text.title, "Image from pics.example.org");
  assert.equal(text.load, "Load");
  assert.equal(text.hint, "Loading fetches it from pics.example.org, so that site sees your address.");
  assert.match(text.failed, /pics\.example\.org/);
  assert.ok(text.loading && text.retry);
});

test("images up to 5 MB load on their own, bigger ones wait for a click", () => {
  assert.equal(MAX_AUTO_IMAGE_BYTES, 5 * 1024 * 1024);
  assert.ok(loadsImageOnItsOwn({ size_bytes: 0 }));
  assert.ok(loadsImageOnItsOwn({ size_bytes: 240_000 }));
  assert.ok(loadsImageOnItsOwn({ size_bytes: MAX_AUTO_IMAGE_BYTES }));
  assert.equal(loadsImageOnItsOwn({ size_bytes: MAX_AUTO_IMAGE_BYTES + 1 }), false);
  assert.equal(loadsImageOnItsOwn({ size_bytes: 19_000_000 }), false);
  assert.equal(loadsImageOnItsOwn({}), false, "an image without a size waits");
  assert.equal(loadsImageOnItsOwn({ size_bytes: -1 }), false);
  assert.equal(loadsImageOnItsOwn({ size_bytes: "lots" }), false);
  assert.equal(loadsImageOnItsOwn(null), false);
});

test("the large image placeholder says its size and why it waits", () => {
  const text = largeImageGateText("12.4 MB");
  assert.equal(text.title, "Image, 12.4 MB");
  assert.match(text.hint, /5 MB/);
  assert.ok(text.load && text.loading && text.retry && text.failed);
});

test("a picture that failed is tried again only after a while", () => {
  const { retriesImageNow, IMAGE_RETRY_AFTER_MS } = require("../scripts/embeds.js");
  assert.equal(retriesImageNow(undefined, 0), true);
  assert.equal(retriesImageNow({ at: 1000 }, 1000), false);
  assert.equal(retriesImageNow({ at: 1000 }, 1000 + IMAGE_RETRY_AFTER_MS - 1), false);
  assert.equal(retriesImageNow({ at: 1000 }, 1000 + IMAGE_RETRY_AFTER_MS), true);
});

test("a missing picture says why in plain words", () => {
  const { missingImageText } = require("../scripts/embeds.js");
  assert.equal(missingImageText("nobody who has this file is online right now").title, "Image unavailable");
  assert.equal(missingImageText("nobody who has this file is online right now").hint, "Nobody who has it is online right now.");
  assert.equal(missingImageText("network is not ready yet").hint, "Commy is still connecting.");
  assert.equal(missingImageText("image too large to preview").hint, "It is too large to show here.");
  assert.equal(missingImageText("that file is larger than 20 MB").hint, "It is too large to show here.");
  assert.equal(missingImageText("not an image").hint, "This file is not a picture Commy can show.");
  assert.equal(missingImageText(undefined).retry, "Try again");
  for (const text of Object.values(missingImageText("io: stream reset by peer"))) assert.doesNotMatch(text, /io:|stream|peer/i);
});

test("big files download straight to disk and show how far along they are", async () => {
  const { BIG_FILE_BYTES, MAX_INLINE_VIDEO_BYTES, isBigFile, downloadProgressText } = require("../scripts/embeds.js");
  assert.equal(BIG_FILE_BYTES, MAX_INLINE_VIDEO_BYTES);
  assert.ok(!isBigFile(BIG_FILE_BYTES));
  assert.ok(isBigFile(BIG_FILE_BYTES + 1));
  assert.ok(!isBigFile(undefined));
  assert.equal(downloadProgressText(0, 0), "Downloading");
  assert.equal(downloadProgressText(0, 200), "Downloading 0%");
  assert.equal(downloadProgressText(99, 200), "Downloading 49%");
  assert.equal(downloadProgressText(200, 200), "Saving");
});

test("a file whose sender no longer has it on this device offers to choose it again", () => {
  const { sentFileGone, missingVideoText, missingImageText } = require("../scripts/embeds.js");
  const gone = "your copy of this file is no longer on this device";
  assert.equal(sentFileGone(gone), true);
  assert.equal(sentFileGone("nobody who has this file is online right now"), false);
  const video = missingVideoText(gone, "nova");
  assert.equal(video.findAgain, true);
  assert.equal(video.waiting, false, "nothing to wait for when the copy was ours");
  assert.equal(video.label, "Your copy of this video is no longer on this device");
  const image = missingImageText(gone);
  assert.equal(image.findAgain, true);
  assert.equal(image.retry, "Choose File");
  assert.equal(missingImageText("nobody who has this file is online right now").findAgain, undefined);
  assert.equal(missingVideoText("nobody who has this file is online right now").findAgain, undefined);
});

test("a video nobody online has says whose app to wait for and keeps trying", () => {
  const { missingVideoText } = require("../scripts/embeds.js");
  const waiting = missingVideoText("nobody who has this file is online right now", "nova");
  assert.equal(waiting.waiting, true);
  assert.equal(waiting.label, "Nobody who has this video is online");
  assert.match(waiting.title, /^Try again when nova or someone who played it is online\./);
  assert.match(missingVideoText("nobody who has this file is online right now").title, /when someone who played it is online/);
  const other = missingVideoText("video too large to play here", "nova");
  assert.equal(other.waiting, false);
  assert.equal(other.label, "Could not load the video: video too large to play here");
});

test("YouTube links get one thumbnail each, from the thumbnail host the account server loads", () => {
  const { youtubeVideoId } = require("../scripts/activities.js");
  const watch = "https://www.youtube.com/watch?v=Sp0ZZuNrXh8";
  const short = "https://youtu.be/Sp0ZZuNrXh8";
  const other = "https://youtu.be/dQw4w9WgXcQ";
  assert.equal(youtubeThumbnail("Sp0ZZuNrXh8"), "https://i.ytimg.com/vi/Sp0ZZuNrXh8/hqdefault.jpg");
  assert.ok(relaysImage(youtubeThumbnail("Sp0ZZuNrXh8")));
  assert.deepEqual(videoLinkEmbeds([watch, short, other, OTHER], youtubeVideoId), [
    { url: watch, thumbnail: youtubeThumbnail("Sp0ZZuNrXh8") },
    { url: other, thumbnail: youtubeThumbnail("dQw4w9WgXcQ") },
  ]);
  assert.deepEqual(videoLinkEmbeds([watch], youtubeVideoId, [short]), [], "a link card with a picture already shows it");
  assert.deepEqual(videoLinkEmbeds([OTHER, "https://example.org/watch?v=Sp0ZZuNrXh8"], youtubeVideoId), []);
  const many = ["a", "b", "c"].map(c => `https://youtu.be/${c.repeat(11)}`);
  assert.equal(videoLinkEmbeds(many, youtubeVideoId).length, MAX_VIDEO_LINK_EMBEDS);
});
