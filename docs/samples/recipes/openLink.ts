export function openInBrowser(href: string): void {
  let url: URL;
  try {
    url = new URL(href);
  } catch {
    return;
  }
  if (url.protocol === "https:" || url.protocol === "http:" || url.protocol === "mailto:") window.open(url.href);
}
