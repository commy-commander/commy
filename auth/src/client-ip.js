import net from "node:net";

export const PRIVATE_NETWORKS = ["127.0.0.0/8", "10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "169.254.0.0/16", "::1/128", "fc00::/7", "fe80::/10"];

const IPV4_MAPPED_PREFIX = "::ffff:";

function plainAddress(address) {
  const trimmed = (address ?? "").trim();
  const unmapped = trimmed.toLowerCase().startsWith(IPV4_MAPPED_PREFIX) ? trimmed.slice(IPV4_MAPPED_PREFIX.length) : "";
  if (net.isIPv4(unmapped)) return unmapped;
  return net.isIP(trimmed) ? trimmed : null;
}

function addRange(list, range) {
  const [address, prefix] = range.split("/");
  const plain = plainAddress(address);
  const family = plain ? `ipv${net.isIP(plain)}` : null;
  const maxPrefix = family === "ipv4" ? 32 : 128;
  const bits = prefix === undefined ? maxPrefix : Number(prefix);
  if (!family || !/^\d*$/.test(prefix ?? "") || !Number.isInteger(bits) || bits < 0 || bits > maxPrefix) {
    throw new Error(`${range} is not an IP address or CIDR range`);
  }
  list.addSubnet(plain, bits, family);
}

export function trustedProxyList(ranges) {
  const list = new net.BlockList();
  for (const range of ranges) addRange(list, range);
  return list;
}

function isTrusted(list, address) {
  return list.check(address, `ipv${net.isIP(address)}`);
}

export function clientIp({ peer, forwardedFor, trustedProxies }) {
  const peerAddress = plainAddress(peer);
  if (!peerAddress || !isTrusted(trustedProxies, peerAddress)) return peerAddress;
  const hops = (forwardedFor ?? "").split(",").map(plainAddress);
  for (let i = hops.length - 1; i >= 0; i--) {
    const hop = hops[i];
    if (!hop) return peerAddress;
    if (!isTrusted(trustedProxies, hop)) return hop;
  }
  return hops.find(Boolean) ?? peerAddress;
}

export function nameTheClient(req, trustedProxies) {
  const forwarded = req.headers["x-forwarded-for"];
  const address = clientIp({
    peer: req.socket?.remoteAddress,
    forwardedFor: Array.isArray(forwarded) ? forwarded.join(",") : forwarded,
    trustedProxies,
  });
  if (address) req.headers["x-forwarded-for"] = address;
  else delete req.headers["x-forwarded-for"];
}
