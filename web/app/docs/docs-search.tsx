"use client";

import Link from "next/link";
import { useMemo, useState } from "react";
import { excerpt, searchDocs, type SearchEntry } from "@/lib/docs-content";

export default function DocsSearch({ entries }: { entries: SearchEntry[] }) {
  const [query, setQuery] = useState("");
  const results = useMemo(() => searchDocs(entries, query), [entries, query]);

  return (
    <div className="docs-search">
      <input
        type="search"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder="Search docs"
        aria-label="Search docs"
      />
      {query.trim() && (
        <ul className="docs-results">
          {results.length === 0 && <li className="muted small">No matches</li>}
          {results.map((r) => (
            <li key={r.slug}>
              <Link href={r.slug === "index" ? "/docs" : `/docs/${r.slug}`} onClick={() => setQuery("")}>
                <strong>{r.title}</strong>
                <span className="muted small">{excerpt(r.text, query)}</span>
              </Link>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
