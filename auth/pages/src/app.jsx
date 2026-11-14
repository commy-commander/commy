import { useEffect, useState } from "react";
import { PAGES } from "./pages/index.js";
import { SiteProvider } from "./site.jsx";

const AS_IS = { display: "contents" };

export function PageApp({ data }) {
  const Page = PAGES[data.page];
  const [ready, setReady] = useState(false);
  useEffect(() => setReady(true), []);
  return (
    <SiteProvider site={data.site}>
      <fieldset className="page-fields" style={AS_IS} disabled={!ready}>
        <Page {...data.props} />
      </fieldset>
    </SiteProvider>
  );
}
