import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { test } from "node:test";

const SRC = new URL("../src/", import.meta.url);
const IDENTIFIER = /^[A-Za-z_$][\w$]*$/;
const CONSTANT = /^[A-Z][A-Z0-9_]*$/;

const BANNED = [
  [/\$(?:query|execute)RawUnsafe\b/, "an unsafe Prisma raw query"],
  [/\bsql\.(?:raw|lit)\s*\(/, "raw SQL text"],
  [/\.unsafe\s*\(/, "an unsafe raw query"],
  [/\.sendCommand\s*\(/, "a raw Redis command"],
  [/\bchild_process\b/, "a shell command"],
  [/operator:\s*["'`](?:contains|starts_with|ends_with)["'`]/, "a LIKE match, which does not escape % and _ in the value"],
];

function lineOf(source, index) {
  return source.slice(0, index).split("\n").length;
}

function skipSpace(source, i) {
  while (/\s/.test(source[i] ?? "")) i++;
  return i;
}

function readQuoted(source, start) {
  const quote = source[start];
  let i = start + 1;
  while (i < source.length && source[i] !== quote) i += source[i] === "\\" ? 2 : 1;
  return i + 1;
}

function readTemplate(source, start) {
  const parts = [];
  let i = start + 1;
  while (i < source.length && source[i] !== "`") {
    if (source[i] === "\\") {
      i += 2;
    } else if (source[i] === "$" && source[i + 1] === "{") {
      let depth = 1;
      let j = i + 2;
      while (j < source.length && depth > 0) {
        if (source[j] === "{") depth++;
        if (source[j] === "}") depth--;
        j++;
      }
      parts.push({ expression: source.slice(i + 2, j - 1).trim(), before: source[i - 1], after: source[j] });
      i = j;
    } else {
      i++;
    }
  }
  return { end: i + 1, parts };
}

function endsArgument(source, i) {
  return [",", ")"].includes(source[skipSpace(source, i)]);
}

function checkQueryText(source, start) {
  const first = source[start];
  if (first === '"' || first === "'") {
    return endsArgument(source, readQuoted(source, start)) ? null : "query text is joined from pieces; pass values as $1, $2 parameters";
  }
  if (first === "`") {
    const { end, parts } = readTemplate(source, start);
    const unquoted = parts.find((p) => !IDENTIFIER.test(p.expression) || p.before !== '"' || p.after !== '"');
    if (unquoted) return `query text interpolates \${${unquoted.expression}} outside a quoted identifier; pass values as $1, $2 parameters`;
    return endsArgument(source, end) ? null : "query text is joined from pieces; pass values as $1, $2 parameters";
  }
  return "query text must be a literal, not a variable or expression";
}

function checkLuaScript(source, start) {
  const name = /^[A-Za-z_$][\w$]*/.exec(source.slice(start))?.[0];
  if (!name || !CONSTANT.test(name) || !endsArgument(source, start + name.length)) {
    return "Redis scripts must be a module constant; pass keys and values as KEYS and ARGV";
  }
  const definition = new RegExp(`const ${name}\\s*=\\s*`).exec(source);
  if (!definition) return `Redis script ${name} must be defined in this file`;
  const at = definition.index + definition[0].length;
  if (source[at] === '"' || source[at] === "'") return null;
  if (source[at] !== "`") return `Redis script ${name} must be a literal`;
  const loose = readTemplate(source, at).parts.find((p) => !CONSTANT.test(p.expression));
  return loose ? `Redis script ${name} interpolates \${${loose.expression}}; pass it as ARGV` : null;
}

function unsafeQueries(source) {
  const found = [];
  for (const [pattern, what] of BANNED) {
    const global = new RegExp(pattern.source, "g");
    for (const m of source.matchAll(global)) found.push({ line: lineOf(source, m.index), problem: `uses ${what}` });
  }
  for (const [call, check] of [
    [/\.query\s*\(/g, checkQueryText],
    [/\.eval\s*\(/g, checkLuaScript],
  ]) {
    for (const m of source.matchAll(call)) {
      const problem = check(source, skipSpace(source, m.index + m[0].length));
      if (problem) found.push({ line: lineOf(source, m.index), problem });
    }
  }
  return found;
}

test("the auth server builds no query text from input", () => {
  const files = readdirSync(SRC, { recursive: true }).filter((f) => /\.(?:m?js|ts)$/.test(f));
  assert.ok(files.length > 0);
  const problems = files.flatMap((file) =>
    unsafeQueries(readFileSync(new URL(file, SRC), "utf8")).map(({ line, problem }) => `src/${file}:${line} ${problem}`),
  );
  assert.deepEqual(problems, []);
});

test("constant and parameterised queries pass", () => {
  const safe = [
    'await pool.query("select 1");',
    "await pool.query('ALTER TABLE IF EXISTS \"linkedDevice\" DROP COLUMN IF EXISTS \"name\"');",
    'await client.query(`SELECT "id", "${field}" AS value FROM "${model}" WHERE "${field}" IS NOT NULL`);',
    'await client.query(`UPDATE "${model}" SET "${field}" = $1 WHERE "id" = $2`, [next, id]);',
    'await client.query(\n  "SELECT 1 WHERE $1",\n  [x],\n);',
    "const SCRIPT = `return redis.call('GET', KEYS[1])`;\nawait client.eval(SCRIPT, { keys: [k], arguments: [v] });",
    "const RING = `if n > ${CAP} then return ${REFUSED} end`;\nclient.eval(RING, { keys: [k] });",
    'adapter.findMany({ model: "invite", where: [{ field: "code", operator: "in", value: codes }] });',
    "const q = ctx.query?.q; const code = ctx.query.code;",
  ];
  for (const source of safe) assert.deepEqual(unsafeQueries(source), [], source);
});

test("query text built from input is refused", () => {
  const unsafe = [
    "pool.query(`select * from \"user\" where email = '${email}'`);",
    "pool.query(`select * from \"user\" where id = ${id}`);",
    'pool.query(`select "${ctx.query.column}" from "user"`);',
    "pool.query(\"select * from users where name = '\" + name + \"'\");",
    "pool.query(text, values);",
    'pool.query(`select 1` + more);',
    "prisma.$queryRawUnsafe(`select ${x}`);",
    "prisma.$executeRawUnsafe(text);",
    "db.selectFrom(sql.raw(input));",
    "client.eval(`return ${input}`, { keys: [] });",
    "client.eval(script, { keys: [] });",
    "const SCRIPT = `return '${token}'`;\nclient.eval(SCRIPT, { keys: [] });",
    'client.sendCommand(["EVAL", body, "0"]);',
    'adapter.findMany({ model: "user", where: [{ field: "name", operator: "contains", value: q }] });',
    'import { exec } from "node:child_process";',
  ];
  for (const source of unsafe) assert.notDeepEqual(unsafeQueries(source), [], source);
});
