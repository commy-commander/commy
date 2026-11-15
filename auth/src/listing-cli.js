import { loadConfig } from "./config.js";
import { createAuth } from "./auth.js";
import { createChecker, oldestFor } from "./listing-checks.js";
import { createListingStore, reportOf, STATUS } from "./listing.js";
import { recheck } from "./listing-rechecks.js";
import { createServerBlocklist } from "./server-blocklist.js";

const USAGE = `Usage: node src/listing-cli.js <command>
  list                      every server that applied, with its status
  show <domain>             one server's details and last check
  recheck <domain>          run the checks now
  approve <domain>          list a server waiting for review
  delist <domain> <reason>  take a server off the list now`;

const originOf = (domain) => `https://${String(domain ?? "").toLowerCase().replace(/^https:\/\//, "").replace(/\/+$/, "")}`;
const day = (value) => (value ? new Date(value).toISOString().slice(0, 10) : "-");

function printRow(row) {
  console.log(`${row.origin}  ${row.status}  ${row.name}`);
  console.log(`  kinds ${row.kinds}  contact ${row.contact ?? "-"}  operator ${row.operatorEmail}  emergency ${row.emergencyEmail}`);
  console.log(`  applied ${day(row.appliedAt)}  listed ${day(row.listedAt)}  checked ${day(row.checkedAt)}  failing since ${day(row.failingSince)}`);
  if (row.reason) console.log(`  reason: ${row.reason}`);
  for (const check of reportOf(row)) console.log(`  ${check.ok ? "pass" : "FAIL"} ${check.id}: ${check.detail}`);
}

async function main([command, domain, ...rest]) {
  if (!command || command === "help") return console.log(USAGE);
  const config = loadConfig();
  if (!config.listing.enabled) throw new Error("the listing is off on this server (LISTING=on turns it on)");
  const blocklist = createServerBlocklist({ listKey: config.listKey, origin: config.baseURL, serverKey: config.serverKey, file: config.serverBlocklistFile, log: { log() {}, warn() {} } });
  await blocklist.refresh();
  const checker = createChecker({ oldest: oldestFor(config), blocked: blocklist.blocks });
  const { auth, pool, send } = createAuth(config, { listing: { checker, onChange() {} } });
  const { adapter } = await auth.$context;
  const store = createListingStore(adapter);
  try {
    if (command === "list") {
      for (const row of (await store.all()).sort((a, b) => a.origin.localeCompare(b.origin))) console.log(`${row.status.padEnd(10)} ${row.origin}  ${row.name}`);
      return;
    }
    const row = await store.find(originOf(domain));
    if (!row) throw new Error(`${originOf(domain)} has not applied`);
    if (command === "show") return printRow(row);
    if (command === "recheck") {
      const outcome = await recheck(adapter, row, { checker, send, instance: config.instanceName, now: Date.now() });
      return printRow({ ...(await store.find(row.origin)), status: outcome.status });
    }
    if (command === "approve") {
      if (row.status !== STATUS.review) throw new Error(`${row.origin} is ${row.status}, not waiting for review`);
      await store.change(row.origin, { status: STATUS.listed, listedAt: new Date() });
      return console.log(`${row.origin} is listed. The served list picks it up within 10 minutes.`);
    }
    if (command === "delist") {
      const reason = rest.join(" ").trim();
      if (!reason) throw new Error("say why, for example: delist chat.example.org \"ignored abuse reports\"");
      await store.change(row.origin, { status: STATUS.delisted, delistedAt: new Date(), reason });
      return console.log(`${row.origin} is delisted. The served list drops it within 10 minutes.`);
    }
    throw new Error(USAGE);
  } finally {
    await pool.end();
  }
}

main(process.argv.slice(2)).catch((err) => {
  console.error(err.message);
  process.exit(1);
});
