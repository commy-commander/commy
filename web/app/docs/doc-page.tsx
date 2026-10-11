import type { Doc } from "@/lib/docs";
import { REPO_URL } from "@/lib/site";

export default function DocPage({ doc }: { doc: Doc }) {
  return (
    <>
      <h1>{doc.title}</h1>
      <div className="docs-body" dangerouslySetInnerHTML={{ __html: doc.html }} />
      <p className="docs-edit muted small">
        <a href={`${REPO_URL}/edit/main/web/content/docs/${doc.slug}.md`} target="_blank" rel="noreferrer">
          Suggest an edit to this page
        </a>
      </p>
    </>
  );
}
