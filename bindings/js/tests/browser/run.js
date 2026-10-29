// SPDX-FileCopyrightText: 2026 KTX Rust port contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//
// Runs gl_upload.html and the tests/webgl demo pages in headless Chrome (WebGL 2 on
// SwiftShader) through chromedriver:
//   CHROMEDRIVER=/path/chromedriver CHROME=/path/chrome-headless-shell node run.js
// The demo pages are served from tests/webgl (with the built modules copied in by `make js`);
// alert(), uncaught errors and console.error are recorded by a script injected into each page.
// Exits 1 when a check fails.
'use strict';

const http = require('http');
const fs = require('fs');
const path = require('path');
const { spawn } = require('child_process');

const ROOT = path.join(__dirname, '..', '..', '..', '..');
const WEBGL = path.join(__dirname, '..', 'webgl');
const TYPES = {
  '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm',
  '.css': 'text/css', '.png': 'image/png',
};

// Records what the page reports; injected first into every served HTML page.
const HOOK = `<script>
  window.__alerts = []; window.__errors = [];
  window.alert = (m) => window.__alerts.push(String(m));
  window.addEventListener('error', (e) => window.__errors.push(String(e.message)));
  window.addEventListener('unhandledrejection', (e) => window.__errors.push(String(e.reason)));
  const consoleError = console.error;
  console.error = (...a) => { window.__errors.push(a.join(' ')); consoleError(...a); };
</script>`;

function serve(root) {
  const server = http.createServer((req, res) => {
    let file = path.join(root, decodeURIComponent(new URL(req.url, 'http://x').pathname));
    if (fs.existsSync(file) && fs.statSync(file).isDirectory()) file = path.join(file, 'index.html');
    if (!file.startsWith(root) || !fs.existsSync(file)) {
      res.writeHead(404).end();
      return;
    }
    res.writeHead(200, { 'Content-Type': TYPES[path.extname(file)] || 'application/octet-stream' });
    if (path.extname(file) === '.html') {
      const html = fs.readFileSync(file, 'utf8');
      const at = html.search(/<script/i);
      res.end(at < 0 ? html : html.slice(0, at) + HOOK + html.slice(at));
    } else {
      fs.createReadStream(file).pipe(res);
    }
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

// Page-side helper: resolves with cond() once truthy (polled), or with null after `ms`.
const WAIT = `const until = (cond, ms = 110000) => new Promise((resolve) => {
  const t0 = Date.now();
  (function poll() {
    let v;
    try { v = cond(); } catch (e) { v = undefined; }
    if (v) resolve(v); else if (Date.now() - t0 > ms) resolve(null); else setTimeout(poll, 50);
  })();
});
const text = (id) => document.getElementById(id).innerText;
const report = (log) => ({ log, alerts: window.__alerts, errors: window.__errors });`;

// Each case: page path, server, and an async script whose result is {log, failed?, ...}.
const CASES = [
  {
    name: 'glUpload',
    root: ROOT,
    url: '/bindings/js/tests/browser/gl_upload.html',
    script: `await until(() => window.__result); return window.__result;`,
  },
  {
    // Upstream tests/webgl/libktx-webgl: click "Run Tests", every result must be PASSED.
    name: 'tests/webgl/libktx-webgl',
    root: WEBGL,
    url: '/libktx-webgl/',
    script: `const log = [];
      const button = document.getElementById('runtests');
      log.push((await until(() => !button.disabled) ? 'ok   ' : 'FAIL ') + 'module ready');
      button.click();
      await until(() => text('compress_astc_result') && button.disabled);
      for (const b of document.querySelectorAll('b[id$="_result"]')) {
        log.push((b.innerText === 'PASSED' ? 'ok   ' : 'FAIL ') + b.id + ': ' + b.innerText);
      }
      for (const item of items) log.push('     ' + item.label.textContent);
      return report(log);`,
  },
  {
    // Upstream tests/webgl/libktx-read-webgl: the initial BasisLZ file, then the UASTC one
    // through replaceTexture (what dropping a file does).
    name: 'tests/webgl/libktx-read-webgl',
    root: WEBGL,
    url: '/libktx-read-webgl/',
    script: `const log = [];
      for (const file of ['ktx_app_blze.ktx2', 'ktx_document_uastc_rdo_5.ktx2']) {
        if (file !== 'ktx_app_blze.ktx2') replaceTexture(gl, file, file);
        const shown = await until(() => text('file_name') === file);
        log.push((shown ? 'ok   ' : 'FAIL ') + file + ' shown, transcoded to ' + text('format'));
        log.push((gl.isTexture(texture.object) ? 'ok   ' : 'FAIL ') + file + ' WebGLTexture');
      }
      return report(log);`,
  },
  {
    // Upstream tests/webgl/llt-three: KTX2Loader on msc_basis_transcoder.js with three.js.
    name: 'tests/webgl/llt-three',
    root: WEBGL,
    url: '/llt-three/',
    network: 'https://unpkg.com/three@0.114.0/build/three.module.js',
    script: `const log = [];
      const format = await until(() => text('format') !== 'FORMAT' && text('format'));
      log.push((format ? 'ok   ' : 'FAIL ') + 'ETC1S transcoded to ' + format);
      return report(log);`,
  },
];

async function online(url) {
  try {
    return (await fetch(url, { method: 'HEAD', signal: AbortSignal.timeout(5000) })).ok;
  } catch {
    return false;
  }
}

async function main() {
  const driverPath = process.env.CHROMEDRIVER || 'chromedriver';
  const driverPort = 9515 + Math.floor(Math.random() * 1000);
  const driver = spawn(driverPath, [`--port=${driverPort}`], { stdio: 'ignore' });
  const servers = new Map();
  let failed = false;
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
        '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist', '--window-size=1280,1024'],
    };
    if (process.env.CHROME) options.binary = process.env.CHROME;
    for (const c of CASES) {
      console.log(`# ${c.name}`);
      if (c.network && !(await online(c.network))) {
        console.log(`skip (${c.network} unreachable)`);
        continue;
      }
      if (!servers.has(c.root)) servers.set(c.root, await serve(c.root));
      const port = servers.get(c.root).address().port;
      const session = await webdriver(driverPort, 'POST', '/session',
        { capabilities: { alwaysMatch: { 'goog:chromeOptions': options } } });
      const id = session.sessionId;
      try {
        await webdriver(driverPort, 'POST', `/session/${id}/timeouts`, { script: 120000 });
        await webdriver(driverPort, 'POST', `/session/${id}/url`,
          { url: `http://127.0.0.1:${port}${c.url}` });
        const result = await webdriver(driverPort, 'POST', `/session/${id}/execute/async`, {
          script: `const done = arguments[0];
            (async () => { ${WAIT} ${c.script} })().then(done,
              (e) => done({ log: ['EXCEPTION ' + (e && e.stack || e)], failed: true }));`,
          args: [],
        });
        for (const line of result.log) console.log(line);
        for (const a of result.alerts || []) console.log(`FAIL alert: ${a}`);
        for (const e of result.errors || []) console.log(`FAIL error: ${e}`);
        if (result.failed || result.log.some((l) => !l.startsWith('ok') && !l.startsWith(' ')) ||
          (result.alerts || []).length || (result.errors || []).length) {
          failed = true;
        }
      } finally {
        await webdriver(driverPort, 'DELETE', `/session/${id}`);
      }
    }
  } finally {
    for (const s of servers.values()) s.close();
    driver.kill();
  }
  console.log(failed ? 'FAILED' : 'PASSED');
  process.exit(failed ? 1 : 0);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
