import { listDocs, searchIndex } from "@/lib/docs";
import DocsNav from "./docs-nav";
import DocsSearch from "./docs-search";

export default function DocsLayout({ children }: { children: React.ReactNode }) {
  const docs = listDocs();
  return (
    <div className="docs-shell">
      <aside className="docs-sidebar">
        <h2>Documentation</h2>
        <DocsSearch entries={searchIndex()} />
        <DocsNav docs={docs} />
      </aside>
      <article className="docs-content">{children}</article>
    </div>
  );
}
