// Rate-limit counters in Redis. Better Auth keys a counter by caller IP and
// path; the key is hashed with the server secret before it reaches Redis, so
// Redis holds a counter per keyed hash that expires with its window and never
// an address. Counters are shared by every auth instance and survive restarts.
import { createHmac } from "node:crypto";

// INCR and the expiry in one step, so concurrent requests cannot all pass a
// stale read. The window starts at the first request and is never extended.
const CONSUME = `
local n = redis.call('INCR', KEYS[1])
if n == 1 then redis.call('PEXPIRE', KEYS[1], ARGV[1]) end
if n <= tonumber(ARGV[2]) then return {1, 0} end
local ttl = redis.call('PTTL', KEYS[1])
if ttl < 0 then
  redis.call('PEXPIRE', KEYS[1], ARGV[1])
  ttl = tonumber(ARGV[1])
end
return {0, ttl}
`;

export function rateLimitKey(secret, key) {
  return `rl:${createHmac("sha256", secret).update(`rate-limit\n${key}`).digest("base64url").slice(0, 22)}`;
}

export function redisRateLimitStorage(redis, secret, { log = console } = {}) {
  let warned = 0;
  return {
    async consume(key, rule) {
      try {
        const [allowed, ttlMs] = await redis.client.eval(CONSUME, {
          keys: [rateLimitKey(secret, key)],
          arguments: [String(rule.window * 1000), String(rule.max)],
        });
        if (Number(allowed) === 1) return { allowed: true, retryAfter: null };
        return { allowed: false, retryAfter: Math.max(1, Math.ceil(Number(ttlMs) / 1000)) };
      } catch (err) {
        // Redis down: let requests through rather than refuse every sign-in.
        // The edge proxy keeps its own limits meanwhile.
        if (Date.now() - warned >= 60_000) {
          warned = Date.now();
          log.warn(`[auth] rate limits are off while redis is unreachable: ${err.message}`);
        }
        return { allowed: true, retryAfter: null };
      }
    },
  };
}
