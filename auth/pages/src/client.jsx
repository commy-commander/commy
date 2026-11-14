import { hydrate, render } from "preact";
import { PageApp } from "./app.jsx";
import ConsentBanner from "./consent-banner.jsx";
import { openConsentSettings } from "./lib/consent.js";

const ROOT_ID = "commy-page";
const DATA_ID = "commy-page-data";
const CONSENT_ID = "commy-consent";

function mount(root, data) {
  hydrate(<PageApp data={data} />, root);
}

function unmount(root) {
  render(null, root);
}

function mountConsent(data) {
  const slot = document.getElementById(CONSENT_ID);
  if (!slot || !data.turnstile) return;
  render(<ConsentBanner host={data.site.host} privacy={data.site.links.privacy} />, slot);
  for (const button of document.querySelectorAll("[data-consent-settings]")) button.addEventListener("click", openConsentSettings);
}

function mountStandalone() {
  const root = document.getElementById(ROOT_ID);
  const source = document.getElementById(DATA_ID);
  if (!root || !source || root.dataset.standalone === undefined) return;
  const data = JSON.parse(source.textContent);
  mount(root, data);
  mountConsent(data);
}

window.commyPages = { mount, unmount };
mountStandalone();
