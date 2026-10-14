import type { Metadata } from "next";
import { getPrivacyPolicy } from "@/lib/privacy";

export const metadata: Metadata = {
  title: "Privacy Policy - Commy",
  description: "How Commy collects, uses and protects your personal information.",
};

export default function PrivacyPage() {
  return (
    <main className="legal">
      <div className="legal-body" dangerouslySetInnerHTML={{ __html: getPrivacyPolicy() }} />
    </main>
  );
}
