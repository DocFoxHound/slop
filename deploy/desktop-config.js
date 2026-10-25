// SPDX-License-Identifier: AGPL-3.0-or-later
// Sets up crates/slop-desktop/tauri.conf.json for one desktop build, before `tauri build`. Shared
// by every build: the Linux packages and the Windows installer built on each server
// (deploy/desktop-linux, deploy/desktop-windows) and production's Windows installer built on
// GitHub (.github/workflows/pipeline.yml).
//
//   VERSION=0.1.N SITE=https://… CHANNEL=production|dev node deploy/desktop-config.js
//
// VERSION   the app's version (0.1.<number of commits>, shared by all platforms)
// SITE      the site the app talks to and updates from
// CHANNEL   production: "Slop", as the file has it (slopai.games's app)
//           dev: "Slop Dev", the test server's app. Its own name, identifier and program name,
//           so it installs beside "Slop" and keeps its own settings, library, sign-in and chat
//           keys (the OS keychain entries are named after the identifier), and its own
//           single-instance lock. deploy/README.md, "The desktop app".
"use strict";
const fs = require("fs");
const path = require("path");

const DEV = {
  productName: "Slop Dev",
  identifier: "dev.ironpoint.slopdev",
  // The program's file name (/usr/bin/<name> in the packages, <name>.exe on Windows), so the
  // two Linux packages don't both install /usr/bin/slop-desktop.
  mainBinaryName: "slop-dev",
};

function configure(conf, { version, site, channel }) {
  if (!/^\d+\.\d+\.\d+$/.test(version || "")) throw new Error(`VERSION must look like 0.1.42, not ${JSON.stringify(version)}`);
  if (!/^https?:\/\/[^/]+$/.test((site || "").replace(/\/$/, ""))) throw new Error(`SITE must be an origin like https://slopai.games, not ${JSON.stringify(site)}`);
  if (channel !== "production" && channel !== "dev") throw new Error(`CHANNEL must be production or dev, not ${JSON.stringify(channel)}`);
  site = site.replace(/\/$/, "");
  conf.version = version;
  conf.bundle.homepage = site;
  conf.plugins.updater.endpoints = conf.plugins.updater.endpoints.map((e) => site + e.slice(e.indexOf("/v1/")));
  // Pictures the app shows: its own, the site's (profile pictures are served by Slop itself),
  // and repositories' (store pages, portfolios, page designs). Nothing else.
  const csp = conf.app.security.csp;
  if (!csp["img-src"].split(" ").includes(site)) csp["img-src"] += ` ${site}`;
  if (channel === "dev") {
    Object.assign(conf, DEV);
    for (const w of conf.app.windows) if (w.title === "Slop") w.title = DEV.productName;
  }
  return conf;
}

if (require.main === module) {
  const file = path.join(__dirname, "..", "crates", "slop-desktop", "tauri.conf.json");
  const conf = JSON.parse(fs.readFileSync(file, "utf8"));
  const { VERSION: version, SITE: site, CHANNEL: channel } = process.env;
  configure(conf, { version, site, channel });
  fs.writeFileSync(file, JSON.stringify(conf, null, 2) + "\n");
  console.log(`desktop-config: ${conf.productName} ${conf.version} (${conf.identifier}) for ${conf.bundle.homepage}`);
}

module.exports = { configure, DEV };
