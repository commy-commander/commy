import Link from "next/link";
import { listDocs, searchIndex } from "@/lib/docs";
import DocsSearch from "./docs-search";

export default function DocsLayout({ children }: { children: React.ReactNode }) {
  const docs = listDocs();
  return (
    <div className="docs-shell">
      <aside className="docs-sidebar">
        <h2>Documentation</h2>
        <DocsSearch entries={searchIndex()} />
        <nav>
          {docs.map((d) => (
            <Link key={d.slug} href={d.slug === "index" ? "/docs" : `/docs/${d.slug}`}>
              {d.nav}
            </Link>
          ))}
        </nav>
      </aside>
      <article className="docs-content">{children}</article>
    </div>
  );
}
