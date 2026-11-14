import { PAGES } from "./pages/index.js";
import { SiteProvider } from "./site.jsx";

export function PageApp({ data }) {
  const Page = PAGES[data.page];
  return (
    <SiteProvider site={data.site}>
      <Page {...data.props} />
    </SiteProvider>
  );
}
