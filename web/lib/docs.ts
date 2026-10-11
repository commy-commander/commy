import fs from "node:fs";
import path from "node:path";
import { Marked } from "marked";
import { parseFrontmatter, plainText, slugify, sortDocs, toMeta, type DocMeta, type SearchEntry } from "./docs-content";

// Docs are markdown files in web/content/docs, rendered at build time.
const DOCS_DIR = path.join(process.cwd(), "content", "docs");

export type Doc = DocMeta & { html: string; text: string };

const marked = new Marked({
  gfm: true,
  renderer: {
    heading({ tokens, depth }) {
      const inner = this.parser.parseInline(tokens);
      const id = slugify(inner);
      return `<h${depth} id="${id}">${inner}</h${depth}>\n`;
    },
    link({ href, title, tokens }) {
      const inner = this.parser.parseInline(tokens);
      const external = /^https?:\/\//.test(href);
      const attrs = external ? ' target="_blank" rel="noreferrer"' : "";
      const t = title ? ` title="${title}"` : "";
      return `<a href="${href}"${t}${attrs}>${inner}</a>`;
    },
  },
});

function readDoc(slug: string): Doc | null {
  const file = path.join(DOCS_DIR, `${slug}.md`);
  if (!/^[a-z0-9-]+$/.test(slug) || !fs.existsSync(file)) return null;
  const { meta, body } = parseFrontmatter(fs.readFileSync(file, "utf8"));
  return { ...toMeta(slug, meta), html: marked.parse(body, { async: false }) as string, text: plainText(body) };
}

export function getDoc(slug: string): Doc | null {
  return readDoc(slug);
}

export function listDocs(): DocMeta[] {
  const slugs = fs
    .readdirSync(DOCS_DIR)
    .filter((f) => f.endsWith(".md"))
    .map((f) => f.slice(0, -3));
  return sortDocs(slugs.map((s) => readDoc(s)).filter((d): d is Doc => d !== null)).map(
    ({ slug, title, nav, description, order }) => ({ slug, title, nav, description, order }),
  );
}

export function searchIndex(): SearchEntry[] {
  return listDocs().map((d) => {
    const doc = readDoc(d.slug)!;
    return { slug: d.slug, title: d.title, description: d.description, text: doc.text.slice(0, 4000) };
  });
}
