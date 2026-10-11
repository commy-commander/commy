// Pure helpers for markdown docs, kept free of fs and Next so they are unit tested.

export type DocMeta = {
  slug: string;
  title: string;
  nav: string;
  description: string;
  order: number;
};

export type ParsedDoc = { meta: Record<string, string>; body: string };

// Minimal frontmatter: `key: value` lines between --- fences.
export function parseFrontmatter(source: string): ParsedDoc {
  const text = source.replace(/^﻿/, "").replace(/\r\n/g, "\n");
  const match = /^---\n([\s\S]*?)\n---\n?/.exec(text);
  if (!match) return { meta: {}, body: text };
  const meta: Record<string, string> = {};
  for (const line of match[1].split("\n")) {
    const i = line.indexOf(":");
    if (i > 0) meta[line.slice(0, i).trim()] = line.slice(i + 1).trim();
  }
  return { meta, body: text.slice(match[0].length) };
}

export function toMeta(slug: string, meta: Record<string, string>): DocMeta {
  const title = meta.title || slug;
  const order = Number(meta.order);
  return {
    slug,
    title,
    nav: meta.nav || title,
    description: meta.description || "",
    order: Number.isFinite(order) ? order : 999,
  };
}

export function sortDocs<T extends DocMeta>(docs: T[]): T[] {
  return [...docs].sort((a, b) => a.order - b.order || a.title.localeCompare(b.title));
}

export function slugify(text: string): string {
  return text
    .toLowerCase()
    .replace(/<[^>]+>/g, "")
    .replace(/&[a-z]+;/g, "")
    .replace(/[^a-z0-9\s-]/g, "")
    .trim()
    .replace(/\s+/g, "-")
    .replace(/-+/g, "-");
}

// Markdown to searchable plain text: drops code fences, link targets and markup.
export function plainText(markdown: string): string {
  return markdown
    .replace(/```[\s\S]*?```/g, " ")
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/[`*_>#|-]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

export type SearchEntry = { slug: string; title: string; description: string; text: string };

export function searchDocs(entries: SearchEntry[], query: string, limit = 8): SearchEntry[] {
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (terms.length === 0) return [];
  const scored = entries
    .map((e) => {
      const title = e.title.toLowerCase();
      const hay = `${title} ${e.description.toLowerCase()} ${e.text.toLowerCase()}`;
      if (!terms.every((t) => hay.includes(t))) return null;
      const score = terms.reduce((s, t) => s + (title.includes(t) ? 10 : 0) + (e.description.toLowerCase().includes(t) ? 3 : 0), 0);
      return { e, score };
    })
    .filter((x): x is { e: SearchEntry; score: number } => x !== null);
  return scored.sort((a, b) => b.score - a.score).slice(0, limit).map((x) => x.e);
}

// A short excerpt around the first match, for search results.
export function excerpt(text: string, query: string, radius = 70): string {
  const term = query.toLowerCase().split(/\s+/).find(Boolean) ?? "";
  const at = term ? text.toLowerCase().indexOf(term) : -1;
  if (at < 0) return text.slice(0, radius * 2) + (text.length > radius * 2 ? "..." : "");
  const start = Math.max(0, at - radius);
  const end = Math.min(text.length, at + term.length + radius);
  return (start > 0 ? "..." : "") + text.slice(start, end) + (end < text.length ? "..." : "");
}
