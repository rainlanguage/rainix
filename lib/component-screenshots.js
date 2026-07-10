#!/usr/bin/env node
"use strict";

// Deterministic Storybook -> PNG renderer for the `component-screenshots`
// reusable workflow (rainlanguage/rainix#262).
//
// The org standard: every screenshot-worthy UI state is a committed CSF story
// that imports the real component/function, and CI renders each story to a
// deterministic PNG so screenshots are regenerable, reviewable, and
// version-controlled (never fabricated in a throwaway DOM harness).
//
// This script is the capture half. The consumer's Storybook is built to a
// static directory first (`storybook build`); this script then:
//   1. enumerates every `story` entry from the built `index.json`
//      (falling back to the legacy `stories.json`);
//   2. serves the static build over loopback HTTP with a deterministic style
//      reset injected into `iframe.html` (animations/transitions/caret killed);
//   3. drives headless Chromium once per story to `iframe.html?id=<id>` and
//      writes `<out>/<id>.png`.
//
// Design choices (kept deliberately dependency-free so the whole thing is
// reproducible under nix with zero npm install and no version-coupled browser
// driver — Chromium comes from `pkgs.chromium`, node from `pkgs.nodejs`):
//   - No Playwright/Puppeteer: Chromium's own `--headless --screenshot` plus
//     `--virtual-time-budget` is the documented deterministic capture path and
//     needs no node driver package.
//   - Determinism: `--virtual-time-budget` advances a virtual clock that waits
//     for pending resource loads (fonts, images) and timers before capturing;
//     the injected style reset removes animations/transitions/caret blink;
//     `--force-color-profile=srgb` and `--font-render-hinting=none` pin colour
//     and glyph rasterisation across machines.
//
// Usage:
//   component-screenshots [--input <dir>] [--out <dir>]
//                         [--width <px>] [--height <px>]
//                         [--virtual-time-budget <ms>]
//
// Exits non-zero if the build has no stories or any capture fails.

const http = require("http");
const fs = require("fs");
const path = require("path");
const os = require("os");
const { spawnSync } = require("child_process");

function parseArgs(argv) {
  const opts = {
    input: "storybook-static",
    out: "screenshots",
    width: 1280,
    height: 800,
    virtualTimeBudget: 10000,
  };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    const next = () => {
      const v = argv[++i];
      if (v === undefined) {
        throw new Error(`missing value for ${arg}`);
      }
      return v;
    };
    switch (arg) {
      case "--input":
        opts.input = next();
        break;
      case "--out":
        opts.out = next();
        break;
      case "--width":
        opts.width = parseInt(next(), 10);
        break;
      case "--height":
        opts.height = parseInt(next(), 10);
        break;
      case "--virtual-time-budget":
        opts.virtualTimeBudget = parseInt(next(), 10);
        break;
      default:
        throw new Error(`unknown argument: ${arg}`);
    }
  }
  return opts;
}

// Read the story ids from a built Storybook. `index.json` (Storybook 7/8) is
// the canonical source; `stories.json` is the legacy fallback. Only `story`
// entries are captured — `docs`/`docsOnly` pages are not component states.
// Ids are sorted so the render order (and any downstream diff) is stable.
function readStoryIds(inputDir) {
  const indexPath = path.join(inputDir, "index.json");
  const legacyPath = path.join(inputDir, "stories.json");

  let ids = [];
  if (fs.existsSync(indexPath)) {
    const index = JSON.parse(fs.readFileSync(indexPath, "utf8"));
    const entries = index.entries || {};
    ids = Object.keys(entries).filter(
      (id) => (entries[id].type || "story") === "story",
    );
  } else if (fs.existsSync(legacyPath)) {
    const legacy = JSON.parse(fs.readFileSync(legacyPath, "utf8"));
    const stories = legacy.stories || {};
    ids = Object.keys(stories).filter(
      (id) => !(stories[id].parameters && stories[id].parameters.docsOnly),
    );
  } else {
    throw new Error(
      `no index.json or stories.json found in ${inputDir}; ` +
        "did the Storybook build run and emit a static directory?",
    );
  }
  return ids.sort();
}

const CONTENT_TYPES = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".map": "application/json; charset=utf-8",
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".gif": "image/gif",
  ".svg": "image/svg+xml",
  ".ico": "image/x-icon",
  ".woff": "font/woff",
  ".woff2": "font/woff2",
  ".ttf": "font/ttf",
  ".otf": "font/otf",
  ".wasm": "application/wasm",
};

// A deterministic reset injected into every `iframe.html` response: kill
// animations, transitions and the text caret so the captured frame does not
// depend on when Chromium happens to sample it.
const STYLE_RESET =
  "<style id=\"rainix-screenshot-reset\">" +
  "*,*::before,*::after{" +
  "animation-duration:0s!important;animation-delay:0s!important;" +
  "transition-duration:0s!important;transition-delay:0s!important;" +
  "caret-color:transparent!important;scroll-behavior:auto!important}" +
  "</style>";

function startServer(inputDir) {
  const root = path.resolve(inputDir);
  const server = http.createServer((req, res) => {
    // Strip the query string, decode, and normalise to a path inside root so
    // `..` traversal cannot escape the served directory.
    const rawPath = decodeURIComponent((req.url || "/").split("?")[0]);
    const relative = path.normalize(rawPath).replace(/^(\.\.[/\\])+/, "");
    let filePath = path.join(root, relative);
    if (filePath.endsWith(path.sep)) {
      filePath = path.join(filePath, "index.html");
    }
    if (!filePath.startsWith(root)) {
      res.statusCode = 403;
      res.end("forbidden");
      return;
    }
    fs.readFile(filePath, (err, data) => {
      if (err) {
        res.statusCode = 404;
        res.end("not found");
        return;
      }
      const ext = path.extname(filePath).toLowerCase();
      res.setHeader(
        "Content-Type",
        CONTENT_TYPES[ext] || "application/octet-stream",
      );
      if (path.basename(filePath) === "iframe.html") {
        let html = data.toString("utf8");
        html = html.includes("<head>")
          ? html.replace("<head>", "<head>" + STYLE_RESET)
          : STYLE_RESET + html;
        res.end(html);
        return;
      }
      res.end(data);
    });
  });
  return new Promise((resolve) => {
    // Port 0 -> an ephemeral free port, read back from the bound address.
    server.listen(0, "127.0.0.1", () => {
      resolve({ server, port: server.address().port });
    });
  });
}

function chromiumBin() {
  return process.env.CHROMIUM_BIN || process.env.CHROME_BIN || "chromium";
}

function captureStory(id, port, outDir, opts) {
  const outPath = path.join(
    outDir,
    id.replace(/[^A-Za-z0-9._-]/g, "-") + ".png",
  );
  const url =
    `http://127.0.0.1:${port}/iframe.html` +
    `?id=${encodeURIComponent(id)}&viewMode=story`;
  // A throwaway profile dir per capture keeps runs hermetic and independent of
  // $HOME. Flags pin colour/glyph rendering and force a fully composited frame
  // before the screenshot is taken.
  const userDataDir = fs.mkdtempSync(path.join(os.tmpdir(), "rainix-shot-"));
  const args = [
    "--headless=new",
    "--no-sandbox",
    "--disable-gpu",
    "--disable-dev-shm-usage",
    "--hide-scrollbars",
    "--force-color-profile=srgb",
    "--font-render-hinting=none",
    "--run-all-compositor-stages-before-draw",
    `--virtual-time-budget=${opts.virtualTimeBudget}`,
    `--user-data-dir=${userDataDir}`,
    `--window-size=${opts.width},${opts.height}`,
    `--screenshot=${outPath}`,
    url,
  ];
  const result = spawnSync(chromiumBin(), args, {
    stdio: ["ignore", "ignore", "inherit"],
    timeout: Math.max(60000, opts.virtualTimeBudget * 4),
  });
  fs.rmSync(userDataDir, { recursive: true, force: true });

  if (result.error) {
    return { id, ok: false, reason: String(result.error) };
  }
  if (result.status !== 0) {
    return { id, ok: false, reason: `chromium exited ${result.status}` };
  }
  if (!fs.existsSync(outPath) || fs.statSync(outPath).size === 0) {
    return { id, ok: false, reason: "no screenshot produced" };
  }
  return { id, ok: true, path: outPath };
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  const inputDir = path.resolve(opts.input);
  const outDir = path.resolve(opts.out);

  const ids = readStoryIds(inputDir);
  if (ids.length === 0) {
    console.error(
      `error: no stories found in ${inputDir}. A frontend repo composing ` +
        "this workflow must commit at least one CSF story.",
    );
    process.exit(1);
  }

  fs.mkdirSync(outDir, { recursive: true });

  const { server, port } = await startServer(inputDir);
  console.log(`Rendering ${ids.length} stories from ${inputDir} -> ${outDir}`);

  const failures = [];
  try {
    for (const id of ids) {
      const result = captureStory(id, port, outDir, opts);
      if (result.ok) {
        console.log(`  ok   ${id}`);
      } else {
        console.error(`  FAIL ${id}: ${result.reason}`);
        failures.push(result);
      }
    }
  } finally {
    server.close();
  }

  if (failures.length > 0) {
    console.error(
      `error: ${failures.length}/${ids.length} stories failed to render`,
    );
    process.exit(1);
  }
  console.log(`Rendered ${ids.length} screenshots to ${outDir}`);
}

main().catch((err) => {
  console.error(err && err.stack ? err.stack : String(err));
  process.exit(1);
});
