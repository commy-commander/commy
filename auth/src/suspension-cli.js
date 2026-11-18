import { loadConfig } from "./config.js";
import { createAuth } from "./auth.js";
import { OPERATOR_USAGE, runOperatorCommand } from "./suspension.js";

async function main(words) {
  if (!words.length || words[0] === "help") return console.log(OPERATOR_USAGE);
  const { auth, pool } = createAuth(loadConfig());
  try {
    process.stdout.write(await runOperatorCommand(await auth.$context, words));
  } finally {
    await pool.end();
  }
}

main(process.argv.slice(2)).catch((err) => {
  console.error(err.message);
  process.exit(1);
});
