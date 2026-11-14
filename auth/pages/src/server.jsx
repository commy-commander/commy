import { renderToString } from "preact-render-to-string";
import { PageApp } from "./app.jsx";
import { PAGES } from "./pages/index.js";

export { createLoaders, isPagePath, PAGE_PATHS, providersFrom } from "./loaders.js";

export function renderPage(data) {
  if (!Object.hasOwn(PAGES, data.page)) throw new Error(`unknown page ${data.page}`);
  return renderToString(<PageApp data={data} />);
}
