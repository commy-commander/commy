import { readFileSync } from "node:fs";

import commonjs from "@rollup/plugin-commonjs";
import nodeResolve from "@rollup/plugin-node-resolve";
import typescript from "@rollup/plugin-typescript";

const sdPlugin = "chat.commy.soundboard.sdPlugin";
const soundTrim = new URL("../desktop/ui/scripts/sound-trim.js", import.meta.url);

export default {
  input: "src/plugin.ts",
  output: {
    file: `${sdPlugin}/bin/plugin.js`,
  },
  plugins: [
    typescript({ include: ["src/**/*.ts"], noEmit: false, allowImportingTsExtensions: false }),
    nodeResolve({ browser: false, exportConditions: ["node"], preferBuiltins: true }),
    commonjs(),
    {
      name: "emit-module-package-file",
      generateBundle() {
        this.emitFile({ fileName: "package.json", source: `{ "type": "module" }`, type: "asset" });
        this.emitFile({ fileName: "sound-trim.js", source: readFileSync(soundTrim, "utf8"), type: "asset" });
      },
    },
  ],
};
