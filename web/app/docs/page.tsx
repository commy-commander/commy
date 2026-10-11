import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { getDoc } from "@/lib/docs";
import DocPage from "./doc-page";

export function generateMetadata(): Metadata {
  const doc = getDoc("index");
  return { title: "Docs - Commy", description: doc?.description };
}

export default function DocsIndexPage() {
  const doc = getDoc("index");
  if (!doc) notFound();
  return <DocPage doc={doc} />;
}
