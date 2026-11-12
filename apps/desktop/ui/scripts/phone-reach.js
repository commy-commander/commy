const REACH_NEVER = ":not(*)";
const REACH_ZOOM = ["maximum-scale=1", "user-scalable=no"];
const REACH_TYPED = 'input, textarea, select, [contenteditable]:not([contenteditable="false"])';
const REACH_FIELDS = [
  [/search/, { enterkeyhint: "search", autocapitalize: "none", autocorrect: "off" }],
  [/^(username-input|add-friend-username|entry-code|entry-code-input|invite-custom-name|template-code-input|activity-link-input)$/, { enterkeyhint: "done", autocapitalize: "none", autocorrect: "off", spellcheck: "false" }],
  [/^(create-name-input|nickname-input|account-display-name|rename-input|rename-folder-input|pet-name|drive-item-name)$/, { enterkeyhint: "done", autocapitalize: "words" }],
  [/^signin-email$/, { enterkeyhint: "next" }],
  [/^(signin-password|signin-code-input|delete-account-code|delete-account-password)$/, { enterkeyhint: "go" }],
];

function hoverless(selector) {
  return selector.replace(/:hover\b/g, REACH_NEVER);
}

function dropHover(rules) {
  let changed = 0;
  for (const rule of rules) {
    if (rule.selectorText && rule.selectorText.includes(":hover")) {
      rule.selectorText = hoverless(rule.selectorText);
      changed += 1;
    }
    if (rule.cssRules && rule.cssRules.length) changed += dropHover(rule.cssRules);
  }
  return changed;
}

function dropHoverIn(sheet) {
  try {
    return sheet && sheet.cssRules ? dropHover(sheet.cssRules) : 0;
  } catch (e) {
    return 0;
  }
}

function lockedViewport(content) {
  const parts = String(content || "").split(",").map(part => part.trim()).filter(Boolean);
  if (parts.some(part => /^width\s*=\s*\d/.test(part))) return parts.join(", ");
  const kept = parts.filter(part => !/^(maximum-scale|user-scalable)\s*=/.test(part));
  const cover = kept.findIndex(part => part.startsWith("viewport-fit"));
  kept.splice(cover < 0 ? kept.length : cover, 0, ...REACH_ZOOM);
  return kept.join(", ");
}

function fieldHints(id) {
  const hints = {};
  for (const [match, set] of REACH_FIELDS) if (match.test(id || "")) Object.assign(hints, set);
  return hints;
}

function dressField(field) {
  for (const [name, value] of Object.entries(fieldHints(field.id))) if (!field.hasAttribute(name)) field.setAttribute(name, value);
}

function typedInto(target) {
  const node = target && target.nodeType === 3 ? target.parentElement : target;
  return Boolean(node && node.closest && node.closest(REACH_TYPED));
}

function wirePhoneReach() {
  const root = document.documentElement;
  if (!root.classList.contains("mobile-app") && !document.body.classList.contains("mobile-app")) return;
  const view = document.querySelector('meta[name="viewport"]');
  if (view) view.setAttribute("content", lockedViewport(view.getAttribute("content")));
  for (const field of document.querySelectorAll("input[id], textarea[id]")) dressField(field);
  if (!touchOnly()) return;
  root.classList.add("touch-only");
  for (const sheet of document.styleSheets) dropHoverIn(sheet);
  document.addEventListener("load", e => {
    if (e.target && e.target.tagName === "LINK") dropHoverIn(e.target.sheet);
  }, true);
  document.addEventListener("dragstart", e => e.preventDefault(), true);
  document.addEventListener("selectstart", e => {
    if (!typedInto(e.target)) e.preventDefault();
  }, true);
}

if (typeof document !== "undefined" && typeof touchOnly === "function") wirePhoneReach();

if (typeof module !== "undefined" && module.exports) {
  module.exports = { REACH_NEVER, REACH_TYPED, REACH_FIELDS, hoverless, dropHover, lockedViewport, fieldHints, typedInto };
}
