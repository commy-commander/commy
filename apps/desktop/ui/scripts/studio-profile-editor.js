const PROFILE_EDITOR_ORDER = ["account-display-name", "account-pronouns", "account-status", "account-presence", "account-about", "account-server-tag"];
const PRESENCE_DOTS = { online: "", idle: "idle", do_not_disturb: "dnd", invisible: "off" };
const EFFECT_SCENE = '<svg class="set-scene theme-dark" viewBox="0 0 352 118" preserveAspectRatio="xMidYMid slice" aria-hidden="true"><rect width="352" height="118" style="fill:var(--accent-deep)"/><rect width="352" height="118" style="fill:var(--accent)" opacity=".35"/><circle cx="268" cy="34" r="19" style="fill:var(--cream)" opacity=".92"/><circle cx="60" cy="22" r="1.4" style="fill:var(--cream)"/><circle cx="118" cy="40" r="1.1" style="fill:var(--cream)"/><circle cx="182" cy="16" r="1.3" style="fill:var(--cream)"/><circle cx="318" cy="70" r="1.1" style="fill:var(--cream)"/><path d="M0 118V74l44-26 40 22 34-16 52 30 46-22 60 28 76-34v62Z" style="fill:var(--bg-darkest)" opacity=".55"/><path d="M0 118V92l58-20 50 16 62-22 70 26 112-30v56Z" style="fill:var(--bg-darkest)"/></svg>';
const EDITOR_KEYS = [
  ["account-avatar-btn", "key sm", "user"],
  ["account-banner-upload", "key sm", "image"],
  ["account-banner-remove", "key sm sq", "x"],
  ["account-about-emoji", "key sq", "smile"],
  ["account-save", "key primary", ""],
];

function presenceDotClass(base, value) {
  const kind = Object.prototype.hasOwnProperty.call(PRESENCE_DOTS, value) ? PRESENCE_DOTS[value] : "";
  return kind ? `${base} ${kind}` : base;
}

function aboutCount(text, limit) {
  return `${String(text || "").length} / ${limit}`;
}

function studioEditorWatch(control, proto, paint) {
  const field = Object.getOwnPropertyDescriptor(proto, "value");
  Object.defineProperty(control, "value", {
    configurable: true,
    get() { return field.get.call(this); },
    set(next) {
      field.set.call(this, next);
      paint(field.get.call(this));
    },
  });
  const repaint = () => paint(field.get.call(control));
  control.addEventListener("input", repaint);
  control.addEventListener("change", repaint);
  repaint();
}

function studioEditorWords(button) {
  if (button.querySelector(":scope > .set-key-words")) return;
  const text = [...button.childNodes].find(node => node.nodeType === 3 && node.textContent.trim());
  if (!text) return;
  const words = document.createElement("span");
  words.className = "set-key-words";
  text.before(words);
  words.appendChild(text);
}

function studioEditorFit() {
  const roomy = document.documentElement.dataset.fit !== "slim";
  const avatarKey = document.getElementById("account-avatar-btn");
  const tools = document.querySelector(".profile-card .role-banner-tools");
  const body = document.querySelector(".profile-card .profile-body");
  const banner = document.getElementById("account-banner");
  if (tools && banner) {
    if (roomy && tools.parentNode === banner) banner.after(tools);
    if (!roomy && tools.parentNode !== banner) banner.appendChild(tools);
  }
  if (avatarKey && tools && body) {
    if (roomy && avatarKey.parentNode !== tools) tools.prepend(avatarKey);
    if (!roomy && avatarKey.parentNode !== body) body.appendChild(avatarKey);
  }
  for (const [id, classes] of EDITOR_KEYS) {
    const key = document.getElementById(id);
    if (key) for (const name of classes.split(" ")) key.classList.toggle(name, roomy);
  }
}

function studioEditorScenes(grid) {
  for (const swatch of grid.querySelectorAll(".look-swatch")) {
    if (!swatch.querySelector(":scope > .set-scene")) swatch.insertAdjacentHTML("afterbegin", EFFECT_SCENE);
  }
}

function studioEditorBanner(strip) {
  if (typeof state === "undefined" || !state.user) return;
  strip.classList.toggle("wash", !state.user.banner);
  if (typeof personTint === "function") strip.style.setProperty("--pc", personTint(state.user.id));
}

function studioProfileEditor() {
  const card = document.querySelector(".settings-page[data-page='account'] .profile-card");
  if (!card || card.classList.contains("p-editor")) return;
  card.classList.add("p-editor");
  const pane = card.closest(".set-pane");
  if (pane) pane.classList.add("set-editor");
  const fields = card.querySelector(".profile-fields");
  if (fields) fields.prepend(...PROFILE_EDITOR_ORDER.map(id => document.getElementById(id)?.closest(".field-label")).filter(Boolean));
  if (typeof studioIconMarkup === "function") {
    for (const [id, , icon] of EDITOR_KEYS) {
      const key = document.getElementById(id);
      if (key && icon && !key.querySelector(":scope > .i")) key.insertAdjacentHTML("afterbegin", studioIconMarkup(icon));
    }
  }
  for (const id of ["account-avatar-btn", "account-save"]) {
    const key = document.getElementById(id);
    if (key) studioEditorWords(key);
  }
  const avatar = document.getElementById("account-avatar");
  if (avatar && !avatar.parentNode.classList.contains("set-avw")) {
    const wrap = document.createElement("span");
    wrap.className = "set-avw";
    avatar.before(wrap);
    const dot = document.createElement("i");
    dot.className = "set-avpres";
    dot.setAttribute("aria-hidden", "true");
    wrap.append(avatar, dot);
  }
  const names = card.querySelector(".profile-names");
  const pronouns = document.getElementById("account-pronouns");
  if (names && pronouns) {
    const line = document.createElement("span");
    line.className = "set-pronouns";
    names.appendChild(line);
    studioEditorWatch(pronouns, HTMLInputElement.prototype, value => { line.textContent = value.trim(); });
  }
  const presence = document.getElementById("account-presence");
  if (presence) {
    const pick = presence.parentNode.classList.contains("set-pick") ? presence.parentNode : null;
    const dot = document.createElement("i");
    dot.setAttribute("aria-hidden", "true");
    if (pick) pick.prepend(dot);
    const avDot = card.querySelector(".set-avpres");
    studioEditorWatch(presence, HTMLSelectElement.prototype, value => {
      dot.className = presenceDotClass("pres-dot", value);
      if (avDot) avDot.className = presenceDotClass("set-avpres", value);
    });
  }
  const about = document.getElementById("account-about");
  if (about) {
    const count = document.createElement("small");
    count.className = "set-count";
    about.after(count);
    studioEditorWatch(about, HTMLTextAreaElement.prototype, value => { count.textContent = aboutCount(value, about.maxLength > 0 ? about.maxLength : 190); });
  }
  const effects = document.getElementById("account-effect-grid");
  if (effects) {
    studioEditorScenes(effects);
    new MutationObserver(() => studioEditorScenes(effects)).observe(effects, { childList: true });
  }
  studioEditorFit();
  new MutationObserver(studioEditorFit).observe(document.documentElement, { attributes: true, attributeFilter: ["data-fit"] });
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { PROFILE_EDITOR_ORDER, PRESENCE_DOTS, presenceDotClass, aboutCount };
}
