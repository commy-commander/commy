import { createCipheriv, createDecipheriv, createHmac, hkdfSync, randomBytes } from "node:crypto";

const PRIVATE = "private";
const LOOKUP = "lookup";
const PRIVATE_PREFIX = "enc1.";
const LOOKUP_PREFIX = "det1.";
const IV_BYTES = 12;
const TAG_BYTES = 16;
const EXACT_MATCHES = new Set(["eq", "ne", "in", "not_in"]);
const NEGATED = new Set(["ne", "not_in"]);

export const SEALED_FIELDS = {
  user: { email: LOOKUP, name: PRIVATE, image: PRIVATE },
  session: { token: LOOKUP },
  account: { accountId: LOOKUP, idToken: PRIVATE },
  verification: { value: PRIVATE },
  passkey: { name: PRIVATE },
  deviceCode: { deviceCode: LOOKUP },
  linkedDevice: { nodeId: LOOKUP },
};

function deriveKey(secret, purpose) {
  return Buffer.from(hkdfSync("sha256", secret, "commy-at-rest-v1", purpose, 32));
}

export function isSealed(value) {
  return typeof value === "string" && (value.startsWith(PRIVATE_PREFIX) || value.startsWith(LOOKUP_PREFIX));
}

export function createSealer(secret) {
  const privateKey = deriveKey(secret, "private fields");
  const lookupKey = deriveKey(secret, "lookup fields");
  const lookupIvKey = deriveKey(secret, "lookup field nonces");

  function encrypt(key, iv, label, text) {
    const cipher = createCipheriv("aes-256-gcm", key, iv);
    cipher.setAAD(Buffer.from(label));
    const body = Buffer.concat([cipher.update(text, "utf8"), cipher.final()]);
    return Buffer.concat([iv, body, cipher.getAuthTag()]).toString("base64url");
  }

  function decrypt(key, label, packed) {
    const raw = Buffer.from(packed, "base64url");
    const decipher = createDecipheriv("aes-256-gcm", key, raw.subarray(0, IV_BYTES));
    decipher.setAAD(Buffer.from(label));
    decipher.setAuthTag(raw.subarray(raw.length - TAG_BYTES));
    return Buffer.concat([decipher.update(raw.subarray(IV_BYTES, raw.length - TAG_BYTES)), decipher.final()]).toString("utf8");
  }

  function lookupIv(label, text) {
    return createHmac("sha256", lookupIvKey).update(label).update("\0").update(text).digest().subarray(0, IV_BYTES);
  }

  return {
    seal(model, field, value) {
      const kind = SEALED_FIELDS[model]?.[field];
      if (!kind || typeof value !== "string") return value;
      const label = `${model}.${field}`;
      return kind === LOOKUP
        ? LOOKUP_PREFIX + encrypt(lookupKey, lookupIv(label, value), label, value)
        : PRIVATE_PREFIX + encrypt(privateKey, randomBytes(IV_BYTES), label, value);
    },
    open(model, field, value) {
      if (!SEALED_FIELDS[model]?.[field] || !isSealed(value)) return value;
      const label = `${model}.${field}`;
      return value.startsWith(LOOKUP_PREFIX)
        ? decrypt(lookupKey, label, value.slice(LOOKUP_PREFIX.length))
        : decrypt(privateKey, label, value.slice(PRIVATE_PREFIX.length));
    },
  };
}

function sealedFieldsOf(model) {
  return SEALED_FIELDS[model] ?? {};
}

function mapFields(sealer, how, model, row) {
  if (!row || typeof row !== "object") return row;
  const fields = sealedFieldsOf(model);
  const out = { ...row };
  for (const field of Object.keys(fields)) {
    if (Object.hasOwn(out, field)) out[field] = sealer[how](model, field, out[field]);
  }
  return out;
}

function openJoined(sealer, model, row, join) {
  const opened = mapFields(sealer, "open", model, row);
  if (!opened || !join) return opened;
  for (const joined of Object.keys(join)) {
    const value = opened[joined];
    if (Array.isArray(value)) opened[joined] = value.map((item) => mapFields(sealer, "open", joined, item));
    else if (value && typeof value === "object") opened[joined] = mapFields(sealer, "open", joined, value);
  }
  return opened;
}

function refuse(model, field, reason) {
  return new Error(`${model}.${field} is encrypted at rest, so it ${reason}`);
}

function sealClause(sealer, model, clause) {
  const kind = sealedFieldsOf(model)[clause.field];
  if (!kind) return clause;
  if (kind !== LOOKUP) throw refuse(model, clause.field, "cannot be searched");
  const operator = clause.operator ?? "eq";
  if (clause.mode === "insensitive" || !EXACT_MATCHES.has(operator)) {
    throw refuse(model, clause.field, "can only be matched exactly");
  }
  const values = Array.isArray(clause.value) ? clause.value : [clause.value];
  if (!values.every((value) => typeof value === "string")) return clause;
  const sealedOrFromAnOlderServer = values.flatMap((value) => {
    const sealed = sealer.seal(model, clause.field, value);
    return isSealed(value) ? [sealed] : [sealed, value];
  });
  return { ...clause, operator: NEGATED.has(operator) ? "not_in" : "in", value: sealedOrFromAnOlderServer };
}

function sealWhere(sealer, model, where) {
  return where?.map((clause) => sealClause(sealer, model, clause));
}

function checkSort(model, sortBy) {
  if (sortBy && sealedFieldsOf(model)[sortBy.field]) throw refuse(model, sortBy.field, "cannot be sorted");
}

export function sealAdapter(adapter, sealer) {
  const seal = (model, data) => mapFields(sealer, "seal", model, data);
  const open = (model, row, join) => openJoined(sealer, model, row, join);
  const wrapped = {
    ...adapter,
    create: async (q) => open(q.model, await adapter.create({ ...q, data: seal(q.model, q.data) })),
    findOne: async (q) => open(q.model, await adapter.findOne({ ...q, where: sealWhere(sealer, q.model, q.where) }), q.join),
    findMany: async (q) => {
      checkSort(q.model, q.sortBy);
      const rows = await adapter.findMany({ ...q, where: sealWhere(sealer, q.model, q.where) });
      return rows.map((row) => open(q.model, row, q.join));
    },
    count: (q) => adapter.count({ ...q, where: sealWhere(sealer, q.model, q.where) }),
    update: async (q) =>
      open(q.model, await adapter.update({ ...q, where: sealWhere(sealer, q.model, q.where), update: seal(q.model, q.update) })),
    updateMany: (q) => adapter.updateMany({ ...q, where: sealWhere(sealer, q.model, q.where), update: seal(q.model, q.update) }),
    delete: (q) => adapter.delete({ ...q, where: sealWhere(sealer, q.model, q.where) }),
    deleteMany: (q) => adapter.deleteMany({ ...q, where: sealWhere(sealer, q.model, q.where) }),
  };
  if (adapter.consumeOne) {
    wrapped.consumeOne = async (q) => open(q.model, await adapter.consumeOne({ ...q, where: sealWhere(sealer, q.model, q.where) }));
  }
  if (adapter.incrementOne) {
    wrapped.incrementOne = async (q) =>
      open(q.model, await adapter.incrementOne({ ...q, where: sealWhere(sealer, q.model, q.where), set: q.set && seal(q.model, q.set) }));
  }
  if (adapter.transaction) {
    wrapped.transaction = (callback) => adapter.transaction((trx) => callback(sealAdapter(trx, sealer)));
  }
  return wrapped;
}

export function sealedAtRest(database, secret) {
  const sealer = createSealer(secret);
  return (options) => sealAdapter(database(options), sealer);
}

async function rewriteColumn(client, model, field, from) {
  const { rows } = await client.query(`SELECT "id", "${field}" AS value FROM "${model}" WHERE "${field}" IS NOT NULL`);
  let changed = 0;
  for (const row of rows) {
    const next = from(row.value);
    if (next === row.value) continue;
    await client.query(`UPDATE "${model}" SET "${field}" = $1 WHERE "id" = $2 AND "${field}" = $3`, [next, row.id, row.value]);
    changed++;
  }
  return changed;
}

async function existingTables(client) {
  const { rows } = await client.query("SELECT table_name FROM information_schema.tables WHERE table_schema = current_schema()");
  return new Set(rows.map((row) => row.table_name));
}

async function rewriteEverySealedColumn(pool, rewrite) {
  const client = await pool.connect();
  try {
    await client.query("BEGIN");
    const tables = await existingTables(client);
    let changed = 0;
    for (const [model, fields] of Object.entries(SEALED_FIELDS)) {
      if (!tables.has(model)) continue;
      for (const field of Object.keys(fields)) changed += await rewriteColumn(client, model, field, (value) => rewrite(model, field, value));
    }
    await client.query("COMMIT");
    return changed;
  } catch (err) {
    await client.query("ROLLBACK").catch(() => {});
    throw err;
  } finally {
    client.release();
  }
}

function opens(sealer, model, field, value) {
  try {
    sealer.open(model, field, value);
    return true;
  } catch {
    return false;
  }
}

export function sealExistingRows(pool, secret) {
  const sealer = createSealer(secret);
  return rewriteEverySealedColumn(pool, (model, field, value) =>
    isSealed(value) && opens(sealer, model, field, value) ? value : sealer.seal(model, field, value),
  );
}

export function openEverySealedRow(pool, secret) {
  const sealer = createSealer(secret);
  return rewriteEverySealedColumn(pool, sealer.open);
}
