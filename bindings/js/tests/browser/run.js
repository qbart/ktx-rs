// SPDX-FileCopyrightText: 2026 KTX Rust port contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//
// Runs gl_upload.html in headless Chrome (WebGL 2 on SwiftShader) through chromedriver:
//   CHROMEDRIVER=/path/chromedriver CHROME=/path/chrome-headless-shell node run.js
// Serves the repository root over HTTP; exits 1 when a check fails.
'use strict';

const http = require('http');
const fs = require('fs');
const path = require('path');
const { spawn } = require('child_process');

const ROOT = path.join(__dirname, '..', '..', '..', '..');
const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm' };

function serve() {
  const server = http.createServer((req, res) => {
    const file = path.join(ROOT, decodeURIComponent(new URL(req.url, 'http://x').pathname));
    if (!file.startsWith(ROOT) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) {
      res.writeHead(404).end();
      return;
    }
    res.writeHead(200, { 'Content-Type': TYPES[path.extname(file)] || 'application/octet-stream' });
    fs.createReadStream(file).pipe(res);
  });
  return new Promise((resolve) => server.listen(0, '127.0.0.1', () => resolve(server)));
}

async function webdriver(port, method, url, body) {
  const r = await fetch(`http://127.0.0.1:${port}${url}`, {
    method,
    headers: { 'Content-Type': 'application/json' },
    body: body && JSON.stringify(body),
  });
  const json = await r.json();
  if (json.value && json.value.error) throw new Error(`${url}: ${json.value.message}`);
  return json.value;
}

async function main() {
  const driverPath = process.env.CHROMEDRIVER || 'chromedriver';
  const driverPort = 9515 + Math.floor(Math.random() * 1000);
  const driver = spawn(driverPath, [`--port=${driverPort}`], { stdio: 'ignore' });
  const server = await serve();
  let failed = true;
  try {
    for (let i = 0; i < 50; i++) {
      try {
        await webdriver(driverPort, 'GET', '/status');
        break;
      } catch {
        await new Promise((r) => setTimeout(r, 100));
      }
    }
    const options = {
      args: ['--headless', '--no-sandbox', '--use-angle=swiftshader',
        '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist'],
    };
    if (process.env.CHROME) options.binary = process.env.CHROME;
    const session = await webdriver(driverPort, 'POST', '/session',
      { capabilities: { alwaysMatch: { 'goog:chromeOptions': options } } });
    const id = session.sessionId;
    try {
      await webdriver(driverPort, 'POST', `/session/${id}/timeouts`, { script: 120000 });
      await webdriver(driverPort, 'POST', `/session/${id}/url`,
        { url: `http://127.0.0.1:${server.address().port}/bindings/js/tests/browser/gl_upload.html` });
      const result = await webdriver(driverPort, 'POST', `/session/${id}/execute/async`, {
        script: `const done = arguments[0];
          (function wait() { window.__result ? done(window.__result) : setTimeout(wait, 50); })();`,
        args: [],
      });
      for (const line of result.log) console.log(line);
      failed = result.failed;
    } finally {
      await webdriver(driverPort, 'DELETE', `/session/${id}`);
    }
  } finally {
    server.close();
    driver.kill();
  }
  console.log(failed ? 'FAILED' : 'PASSED');
  process.exit(failed ? 1 : 0);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
