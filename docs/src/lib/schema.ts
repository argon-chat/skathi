import logo from "../../../logos/logo_single.png?url";

export type Thing = Record<string, unknown>;

export const organizationId = (site: URL) => new URL("/#organization", site).href;
export const websiteId = (site: URL) => new URL("/#website", site).href;

/** The nodes every page shares; a page's own nodes refer to them by `@id`. */
export function siteGraph(site: URL): Thing[] {
  return [
    {
      "@type": "Organization",
      "@id": organizationId(site),
      name: "Argon Inc. LLC",
      url: site.href,
      logo: new URL(logo, site).href,
      sameAs: ["https://github.com/argon-chat"],
    },
    {
      "@type": "WebSite",
      "@id": websiteId(site),
      name: "Skathi",
      url: site.href,
      inLanguage: "en",
      publisher: { "@id": organizationId(site) },
    },
  ];
}
