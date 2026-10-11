import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { getDoc, listDocs } from "@/lib/docs";
import DocPage from "../doc-page";

type Props = { params: { slug: string } };

export const dynamicParams = false;

export function generateStaticParams() {
  return listDocs()
    .filter((d) => d.slug !== "index")
    .map((d) => ({ slug: d.slug }));
}

export function generateMetadata({ params }: Props): Metadata {
  const doc = getDoc(params.slug);
  return doc ? { title: `${doc.title} - Commy Docs`, description: doc.description } : {};
}

export default function DocSlugPage({ params }: Props) {
  const doc = params.slug === "index" ? null : getDoc(params.slug);
  if (!doc) notFound();
  return <DocPage doc={doc} />;
}
