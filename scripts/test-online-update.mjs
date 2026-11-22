// Uses the existing 0.7.12 signed release fixture and an ephemeral HTTPS server.
// The test trusts its certificate in memory; it does not change Windows trust settings.
import { readFile, mkdtemp, rm } from 'node:fs/promises';
import { spawn, execFileSync } from 'node:child_process';
import { once } from 'node:events';
import { createServer } from 'node:https';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const tauri = fileURLToPath(new URL('../src-tauri/', import.meta.url));
const fixtureName = 'Luma_0.7.12_x64-setup.exe';
const fixture = process.argv[2] ?? join(tauri, 'target/release/bundle/nsis', fixtureName);
const installer = await readFile(fixture);
const signature = (await readFile(`${fixture}.sig`, 'utf8')).trim();
const dir = await mkdtemp(join(tmpdir(), 'luma-update-test-'));
const cert = join(dir, 'cert.pem');
const key = join(dir, 'key.pem');
let server;
try {
  execFileSync('openssl', ['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1', '-subj', '/CN=localhost', '-addext', 'subjectAltName=IP:127.0.0.1', '-addext', 'basicConstraints=critical,CA:FALSE', '-keyout', key, '-out', cert], { stdio: 'pipe' });
  server = createServer({ cert: await readFile(cert), key: await readFile(key) }, (req, res) => {
    switch (req.url) {
      case '/latest.json':
        return res.end(JSON.stringify({ version: '0.7.12', signature }));
      case `/${fixtureName}`:
        // Exercise the same HTTPS-to-HTTPS redirect pattern as GitHub Release assets.
        return res.writeHead(302, { location: '/artifact' }).end();
      case '/artifact':
        return res.end(installer);
      case '/large':
        res.write('1234');
        return setImmediate(() => res.end('56789'));
      case '/downgrade':
        return res.writeHead(302, { location: `http://127.0.0.1:${server.address().port}/artifact` }).end();
      default:
        return res.writeHead(404).end();
    }
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const child = spawn('cargo', ['test', 'downloads_from_https_fixture', '--', '--ignored'], {
    cwd: tauri,
    stdio: 'inherit',
    env: {
      ...process.env,
      RUSTUP_TOOLCHAIN: 'stable-x86_64-pc-windows-gnullvm',
      LUMA_TEST_FEED: `https://127.0.0.1:${server.address().port}/latest.json`,
      LUMA_TEST_CERT: cert,
      LUMA_TEST_INSTALLER: fixture,
    },
  });
  const [code] = await once(child, 'exit');
  process.exitCode = code ?? 1;
} finally {
  server?.close();
  await rm(dir, { recursive: true, force: true });
}
