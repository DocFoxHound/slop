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
// CHANNEL   production: "Slop", as the file has it (slaporslop.games's app)
//           dev: "Slop Dev", the test server's app. Its own name, identifier and program name,
//           so it installs beside "Slop" and keeps its own settings, library, sign-in and chat
//           keys (the OS keychain entries are named after the identifier), and its own
//           single-instance lock. deploy/README.md, "The desktop app".
//
// Those are the product names the installers and packages are keyed by, so they stay "Slop" and
// "Slop Dev" after the rename to SLAPorSLOP (changing them would install a second copy instead
// of updating). The window titles, the Windows shortcuts (crates/slop-desktop/windows/installer.nsi)
// and the Linux menu entries (crates/slop-desktop/linux/*.desktop) say "SLAPorSLOP" and
// "SLAPorSLOP Dev".
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
const DEV_TITLE = "SLAPorSLOP Dev";
// The Linux menu entry for the test server's app.
const DEV_DESKTOP_ENTRY = "linux/slaporslop-dev.desktop";
// The Tauri CLI that windows/installer.nsi was taken from (tauri-bundler 2.10.3). Another version
// may have changed Tauri's own template, so the build stops until that file is brought up to date.
const TEMPLATE_CLI = "2.12.3";

function tauriCli() {
  const lock = JSON.parse(fs.readFileSync(path.join(__dirname, "..", "web", "package-lock.json"), "utf8"));
  return lock.packages["node_modules/@tauri-apps/cli"].version;
}

function configure(conf, { version, site, channel }) {
  if (!/^\d+\.\d+\.\d+$/.test(version || "")) throw new Error(`VERSION must look like 0.1.42, not ${JSON.stringify(version)}`);
  if (!/^https?:\/\/[^/]+$/.test((site || "").replace(/\/$/, ""))) throw new Error(`SITE must be an origin like https://slaporslop.games, not ${JSON.stringify(site)}`);
  if (channel !== "production" && channel !== "dev") throw new Error(`CHANNEL must be production or dev, not ${JSON.stringify(channel)}`);
  site = site.replace(/\/$/, "");
  conf.version = version;
  conf.bundle.homepage = site;
  conf.plugins.updater.endpoints = conf.plugins.updater.endpoints.map((e) => site + e.slice(e.indexOf("/v1/")));
  // Pictures the app shows: its own, the site's (profile pictures are served by SLAPorSLOP itself),
  // and repositories' (store pages, portfolios, page designs). Nothing else.
  const csp = conf.app.security.csp;
  if (!csp["img-src"].split(" ").includes(site)) csp["img-src"] += ` ${site}`;
  if (channel === "dev") {
    Object.assign(conf, DEV);
    for (const w of conf.app.windows) if (w.title === "SLAPorSLOP") w.title = DEV_TITLE;
    for (const kind of ["deb", "rpm"]) conf.bundle.linux[kind].desktopTemplate = DEV_DESKTOP_ENTRY;
  }
  return conf;
}

if (require.main === module) {
  const file = path.join(__dirname, "..", "crates", "slop-desktop", "tauri.conf.json");
  const conf = JSON.parse(fs.readFileSync(file, "utf8"));
  const cli = tauriCli();
  if (cli !== TEMPLATE_CLI) {
    throw new Error(
      `@tauri-apps/cli is ${cli}, but crates/slop-desktop/windows/installer.nsi is Tauri's template for ${TEMPLATE_CLI}: ` +
        "update it from the new tauri-bundler (see the top of that file), then TEMPLATE_CLI here",
    );
  }
  const { VERSION: version, SITE: site, CHANNEL: channel } = process.env;
  configure(conf, { version, site, channel });
  fs.writeFileSync(file, JSON.stringify(conf, null, 2) + "\n");
  console.log(`desktop-config: ${conf.productName} ${conf.version} (${conf.identifier}) for ${conf.bundle.homepage}`);
}

module.exports = { configure, DEV, DEV_TITLE, DEV_DESKTOP_ENTRY, TEMPLATE_CLI };
