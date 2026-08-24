#!/usr/bin/env node
// Postinstall: download the platform binary from the matching GitHub release
// and verify its SHA-256 against the release's SHA256SUMS manifest (which is
// itself Ed25519-signed — see the repo README for full verification).
"use strict";

const { execFileSync } = require("child_process");
const crypto = require("crypto");
const fs = require("fs");
const https = require("https");
const path = require("path");

const REPO = "eniyos/backbeatin";
const pkg = JSON.parse(
  fs.readFileSync(path.join(__dirname, "package.json"), "utf8")
);
const VERSION = `v${pkg.version}`;

const PLATFORMS = {
  "darwin-x64": "backbeatin-macos-x86_64",
  "darwin-arm64": "backbeatin-macos-aarch64",
  "linux-x64": "backbeatin-linux-x86_64",
  "linux-arm64": "backbeatin-linux-aarch64",
};

const key = `${process.platform}-${process.arch}`;
const artifact = PLATFORMS[key];
if (!artifact) {
  console.error(
    `backbeatin: unsupported platform ${key}. ` +
      `Install from GitHub releases instead: https://github.com/${REPO}/releases`
  );
  process.exit(1);
}

const base = `https://github.com/${REPO}/releases/download/${VERSION}`;
const tarball = `${artifact}.tar.gz`;
const targetDir = path.join(__dirname, "backbeatin-bin");

function fetch(url) {
  return new Promise((resolve, reject) => {
    https
      .get(url, { headers: { "User-Agent": "backbeatin-installer" } }, (res) => {
        if ([301, 302, 307, 308].includes(res.statusCode)) {
          res.resume();
          fetch(res.headers.location).then(resolve, reject);
          return;
        }
        if (res.statusCode !== 200) {
          reject(new Error(`HTTP ${res.statusCode} for ${url}`));
          res.resume();
          return;
        }
        const chunks = [];
        res.on("data", (c) => chunks.push(c));
        res.on("end", () => resolve(Buffer.concat(chunks)));
        res.on("error", reject);
      })
      .on("error", reject);
  });
}

async function main() {
  const [tarBytes, sumsBytes] = await Promise.all([
    fetch(`${base}/${tarball}`),
    fetch(`${base}/SHA256SUMS`),
  ]);

  // Verify the tarball against the signed SHA256SUMS manifest before
  // extracting anything.
  const actual = crypto.createHash("sha256").update(tarBytes).digest("hex");
  const expectedLine = sumsBytes
    .toString("utf8")
    .split("\n")
    .find((l) => l.trimEnd().endsWith(tarball));
  const expected = expectedLine && expectedLine.split(/\s+/)[0];
  if (!expected || expected.toLowerCase() !== actual.toLowerCase()) {
    throw new Error(
      `SHA-256 mismatch for ${tarball}: expected ${expected ?? "(not listed)"}, got ${actual}`
    );
  }

  fs.rmSync(targetDir, { recursive: true, force: true });
  fs.mkdirSync(targetDir, { recursive: true });
  const tmpTar = path.join(targetDir, tarball);
  fs.writeFileSync(tmpTar, tarBytes);
  execFileSync("tar", ["xzf", tmpTar, "-C", targetDir], { stdio: "inherit" });
  fs.rmSync(tmpTar);
  fs.chmodSync(path.join(targetDir, "backbeatin"), 0o755);
  console.log(
    `backbeatin ${VERSION} installed (${artifact}) — checksum verified against SHA256SUMS.`
  );
}

main().catch((err) => {
  console.error(`backbeatin install failed: ${err.message}`);
  process.exit(1);
});
