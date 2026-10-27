const TIP_JAR_HOST = "ko-fi.com";
const TIP_JAR_MAX_PAGE = 64;

function cleanTipJar(input) {
  const text = String(input || "").trim();
  let rest = text.replace(/^https?:\/\//, "");
  if (rest.startsWith("www.")) rest = rest.slice(4);
  let page;
  const slash = rest.indexOf("/");
  if (slash >= 0) {
    if (rest.slice(0, slash).toLowerCase() !== TIP_JAR_HOST) return null;
    page = rest.slice(slash + 1).replace(/\/+$/, "");
  } else {
    if (rest.includes(".") || rest.includes(":")) return null;
    page = rest;
  }
  if (!page || page.length > TIP_JAR_MAX_PAGE || !/^[A-Za-z0-9_-]+$/.test(page)) return null;
  return `https://${TIP_JAR_HOST}/${page}`;
}

function tipJarLabel(url) {
  return url ? url.replace(/^https:\/\//, "") : "";
}

if (typeof module !== "undefined") {
  module.exports = { TIP_JAR_HOST, TIP_JAR_MAX_PAGE, cleanTipJar, tipJarLabel };
}
