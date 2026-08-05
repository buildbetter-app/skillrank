import { createServer } from "node:http";
for (const k of ["KV_REST_API_URL", "KV_REST_API_TOKEN", "UPSTASH_REDIS_REST_URL", "UPSTASH_REDIS_REST_TOKEN"]) delete process.env[k];
process.env.EVAL_MAX_BODY_BYTES = "512";
process.env.REGISTRY_ADMIN_TOKEN = 'Xk9#mQ2$vL8@nP4!';
const { default: handler } = await import("../api/registry.mjs");
const server = createServer((req, res) => handler(req, res));
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const base = `http://127.0.0.1:${server.address().port}`;
const call = async (path, init = {}) => {
  try {
    const res = await fetch(`${base}/api/registry?${new URLSearchParams({ path })}`, init);
    return { status: res.status, body: await res.text() };
  } catch (e) {
    return { error: `${e.name}: ${e.message}${e.cause ? " / " + e.cause.message : ""}` };
  }
};

// F12a: over EVAL_MAX_BODY_BYTES but under the hard cap
console.log("F12a small overflow:", JSON.stringify(await call("eval-results", { method: "POST", body: "x".repeat(4096) })).slice(0, 200));

// F12b: over HARD_BODY_CAP (4 MiB) — the path that settles early
console.log("F12b hard-cap overflow:", JSON.stringify(await call("eval-results", { method: "POST", body: "x".repeat(5 * 1024 * 1024) })).slice(0, 300));

// F15: admin credential outside the registry-token charset
console.log("F15 correct admin cred:", JSON.stringify(await call("auth/accounts/acct_aaaaaaaaaa/revoke", { method: "POST", headers: { Authorization: 'Bearer Xk9#mQ2$vL8@nP4!' } })).slice(0, 200));
console.log("F15 wrong admin cred:  ", JSON.stringify(await call("auth/accounts/acct_aaaaaaaaaa/revoke", { method: "POST", headers: { Authorization: "Bearer wrong-credential-here" } })).slice(0, 200));
server.close();
