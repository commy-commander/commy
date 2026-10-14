"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import type { DocMeta } from "@/lib/docs-content";

export default function DocsNav({ docs }: { docs: DocMeta[] }) {
  const pathname = usePathname();
  return (
    <nav aria-label="Documentation">
      {docs.map((d) => {
        const href = d.slug === "index" ? "/docs" : `/docs/${d.slug}`;
        return (
          <Link key={d.slug} href={href} aria-current={pathname === href ? "page" : undefined}>
            {d.nav}
          </Link>
        );
      })}
    </nav>
  );
}
