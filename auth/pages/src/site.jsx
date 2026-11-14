import { createContext } from "preact";
import { useContext } from "preact/hooks";

const SiteContext = createContext({ name: "Commy", host: "", links: {} });

export function SiteProvider({ site, children }) {
  return <SiteContext.Provider value={site}>{children}</SiteContext.Provider>;
}

export function useSite() {
  return useContext(SiteContext);
}
