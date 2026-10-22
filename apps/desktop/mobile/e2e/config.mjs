export const DEFAULTS = {
  package: "chat.commy.mobile",
  tabletDebugPort: 9333,
  pushGapSeconds: 15,
  scenarios: ["launch", "accounts", "wake", "burst", "badge", "delivery"],
  bench: { debugPort: 9334, attach: false },
};

export function parseArgs(argv) {
  const args = { only: null, apk: null, run: null, config: null, report: null };
  for (let i = 0; i < argv.length; i++) {
    const flag = argv[i];
    const value = () => {
      const next = argv[++i];
      if (next === undefined || next.startsWith("--")) throw new Error(`${flag} needs a value`);
      return next;
    };
    if (flag === "--config") args.config = value();
    else if (flag === "--apk") args.apk = value();
    else if (flag === "--run") args.run = value();
    else if (flag === "--only") args.only = value().split(",").map(s => s.trim()).filter(Boolean);
    else if (flag === "--report") args.report = value();
    else throw new Error(`unknown option ${flag}`);
  }
  if (args.apk && args.run) throw new Error("use --apk or --run, not both");
  return args;
}

export function resolveConfig(file, args, known) {
  const config = { ...DEFAULTS, ...file, bench: { ...DEFAULTS.bench, ...(file.bench ?? {}) } };
  const scenarios = args.only ?? config.scenarios;
  const unknown = scenarios.filter(name => !known.includes(name));
  if (unknown.length) throw new Error(`unknown scenario: ${unknown.join(", ")} (known: ${known.join(", ")})`);
  const needsBench = scenarios.some(name => name !== "launch" && name !== "badge");
  if (needsBench && !config.bench.attach && (!config.bench.exe || !config.bench.dataDir)) {
    throw new Error("bench.exe and bench.dataDir are needed for these scenarios, or set bench.attach with a running bench's debugPort");
  }
  return { ...config, scenarios, needsBench };
}

export function summarize(results) {
  const lines = results.map(r => {
    const mark = r.ok ? "PASS" : r.skipped ? "SKIP" : "FAIL";
    const extra = r.error ?? (r.details?.reason ? r.details.reason : "");
    return `${mark}  ${r.name.padEnd(9)} ${(r.ms / 1000).toFixed(1).padStart(6)}s  ${extra}`.trimEnd();
  });
  const failed = results.filter(r => !r.ok && !r.skipped).length;
  lines.push(failed ? `${failed} of ${results.length} failed` : `all ${results.length} passed`);
  return { text: lines.join("\n"), failed };
}
