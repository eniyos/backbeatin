#!/usr/bin/env node
// Thin launcher: forwards to the native binary downloaded by install.js.
"use strict";

const { spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");

const binary = path.join(__dirname, "..", "backbeatin-bin", "backbeatin");

if (!fs.existsSync(binary)) {
  console.error(
    "backbeatin: native binary not found (postinstall may have been skipped).\n" +
      "Reinstall with lifecycle scripts enabled, e.g.:\n" +
      "  bun install -g backbeatin   (bun runs postinstall by default)\n" +
      "  npm install -g backbeatin --foreground-scripts"
  );
  process.exit(1);
}

const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });
if (result.error) {
  console.error(`backbeatin: failed to start: ${result.error.message}`);
  process.exit(1);
}
process.exit(result.status === null ? 1 : result.status);
