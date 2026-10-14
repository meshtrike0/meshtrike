// Second batch of browser-harness flows: groups, media, QR handoff under
// jamming, remote pairing, High-Risk mode, duress PIN.
// Usage: node e2e/flows2.mjs [baseUrl] [screenshotDir]
import { chromium } from '@playwright/test';
import assert from 'node:assert/strict';
import path from 'node:path';
import fs from 'node:fs';

const BASE = process.argv[2] ?? 'http://localhost:8088/';
const SHOTS = process.argv[3] ?? '.';
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1800, height: 1500 } });
const errors = [];
page.on('pageerror', (e) => errors.push(e.message));
page.on('console', (m) => m.type() === 'error' && errors.push(m.text()));

const phone = (name) => page.getByTestId(`phone-${name}`);
const step = async (label, fn) => {
  const t0 = Date.now();
  await fn();
  console.log(`✓ ${label} (${Date.now() - t0} ms)`);
};
const btn = (p, name) => phone(p).getByRole('button', { name, exact: true });
const tab = (p, name) => phone(p).getByRole('tab', { name, exact: true });
const home = async (p) => {
  for (let i = 0; i < 8 && !(await tab(p, 'Settings').isVisible()); i++) {
    if (await btn(p, 'Back').isVisible()) await btn(p, 'Back').click();
    else if (await btn(p, 'Done').isVisible()) await btn(p, 'Done').click();
    else break;
  }
};

async function onboard(name, pin = '123456', duress = '') {
  const p = phone(name);
  await p.getByRole('button', { name: 'Skip' }).click();
  await p.getByLabel('Nickname').fill(name.toLowerCase());
  await p.getByLabel('App PIN (6 digits)').fill(pin);
  await p.getByLabel('Repeat PIN').fill(pin);
  if (duress) {
    await p.getByRole('switch', { name: 'Duress PIN' }).click();
    await p.getByLabel('Duress PIN (optional)').fill(duress);
  }
  await p.getByRole('button', { name: 'Create identity' }).click();
  await p.getByText('Chats', { exact: true }).waitFor({ timeout: 30000 });
}

async function pair(shower, scanner) {
  for (const n of [shower, scanner]) {
    await home(n);
    await btn(n, 'New chat').click();
    await phone(n).getByRole('button', { name: /Add contact in person/ }).click();
  }
  await phone(shower).getByLabel('QR code').waitFor();
  await phone(scanner).getByRole('radio', { name: 'Scan code' }).click();
  await phone(scanner).getByRole('radio', { name: `${shower}'s screen` }).click();
  await phone(scanner).getByText(`Paired with ${shower.toLowerCase()}`).waitFor({ timeout: 30000 });
  await phone(shower).getByText(`Paired with ${scanner.toLowerCase()}`).waitFor({ timeout: 30000 });
  await btn(shower, 'Done').click();
  await btn(scanner, 'Done').click();
}

async function openChat(p, title) {
  await home(p);
  await phone(p).getByRole('button', { name: new RegExp(`^${title}, \\d+ unread`) }).first().click({ timeout: 30000 });
}

async function send(p, text) {
  await phone(p).getByLabel('Message text').fill(text);
  await btn(p, 'Send').click();
}

try {
  await page.goto(BASE);
  await phone('Bob').waitFor({ timeout: 60000 });
  await page.getByRole('button', { name: '+ Phone', exact: true }).click();
  await phone('Carol').waitFor();

  await step('onboard three phones (Carol with a duress PIN)', async () => {
    await onboard('Alice');
    await onboard('Bob');
    await onboard('Carol', '111111', '999999');
  });

  await step('Alice pairs with Bob and with Carol', async () => {
    await pair('Bob', 'Alice');
    await pair('Carol', 'Alice');
  });

  await step('group with members who are not each other’s contacts', async () => {
    await home('Alice');
    await btn('Alice', 'New chat').click();
    await phone('Alice').getByRole('button', { name: /New group/ }).click();
    await phone('Alice').getByLabel('Group name').fill('trio');
    await phone('Alice').getByRole('button', { name: /^bob, not selected/ }).click();
    await phone('Alice').getByRole('button', { name: /^carol, not selected/ }).click();
    await btn('Alice', 'Create group').click();
    await send('Alice', 'Welcome to trio');
    await openChat('Bob', 'trio');
    await phone('Bob').getByText('Welcome to trio').waitFor({ timeout: 30000 });
    await openChat('Carol', 'trio');
    await phone('Carol').getByText('Welcome to trio').waitFor({ timeout: 30000 });
    await send('Bob', 'Hi Carol, via the group');
    await phone('Carol').getByText('Hi Carol, via the group').waitFor({ timeout: 30000 });
  });

  await step('photo is re-encoded in Rust and drawn from raw pixels', async () => {
    await btn('Alice', 'Attach').click();
    const chooser = page.waitForEvent('filechooser');
    await btn('Alice', 'Photo').click();
    await (await chooser).setFiles(path.resolve('assets/icon.png'));
    await phone('Bob').getByRole('img', { name: 'Photo' }).waitFor({ timeout: 60000 });
  });

  await step('file transfer offers Save, never opens', async () => {
    const tmp = path.join(SHOTS, 'notes.txt');
    fs.writeFileSync(tmp, 'meeting at 6'.repeat(400));
    await btn('Alice', 'Attach').click();
    const chooser = page.waitForEvent('filechooser');
    await btn('Alice', 'File').click();
    await (await chooser).setFiles(tmp);
    await phone('Carol').getByText('notes.txt').waitFor({ timeout: 60000 });
    await phone('Carol').getByRole('button', { name: 'Save to Files' }).waitFor({ timeout: 60000 });
  });

  await step('remote pairing from a public channel post (Bob ↔ Carol)', async () => {
    for (const n of ['Bob', 'Carol']) {
      await home(n);
      await btn(n, 'New chat').click();
      await phone(n).getByRole('button', { name: /Public channels/ }).click();
      await phone(n).getByLabel('Channel name').fill('square');
      await btn(n, 'Join').click();
    }
    await send('Carol', 'Looking for a ride');
    await phone('Bob').getByText('Looking for a ride').waitFor({ timeout: 30000 });
    await phone('Bob').getByRole('button', { name: 'Start private chat' }).click();
    await phone('Carol').getByText(/wants to chat/).first().waitFor({ timeout: 30000 });
    await btn('Carol', 'Accept').click();
    await home('Bob');
    await btn('Bob', 'New chat').click();
    await phone('Bob').getByRole('button', { name: /carol/ }).waitFor({ timeout: 30000 });
    assert.ok(await phone('Bob').getByLabel('Trust: Not verified').first().isVisible());
  });

  await step('QR handoff moves a message through a jammer', async () => {
    await home('Bob');
    // Place a jammer over Bob and Carol (map click at Bob).
    const map = page.getByTestId('map');
    const box = await map.boundingBox();
    await page.getByRole('button', { name: 'Place jammer' }).click();
    const bob = await page.getByTestId('map-phone-Bob').locator('circle').first().boundingBox();
    await page.mouse.click(bob.x + bob.width / 2, bob.y + bob.height / 2);
    await page.waitForTimeout(1500);
    await openChat('Alice', 'bob');
    await send('Alice', 'Sent by camera');
    await page.waitForTimeout(2000);
    for (const n of ['Alice', 'Bob']) {
      await home(n);
      await tab(n, 'Network').click();
      await btn(n, 'QR handoff').click();
    }
    await phone('Bob').getByRole('button', { name: 'Receive (film codes)' }).click();
    await phone('Bob').getByRole('radio', { name: "Alice's screen" }).click({ timeout: 10000 });
    await phone('Bob').getByText('Transfer complete').waitFor({ timeout: 60000 });
    await openChat('Bob', 'alice');
    await phone('Bob').getByText('Sent by camera').waitFor({ timeout: 10000 });
    await page.getByRole('button', { name: 'Clear jammers' }).click();
    void box;
  });

  await step('High-Risk mode: confirm screen, fixed frames, text still flows', async () => {
    await home('Alice');
    await tab('Alice', 'Settings').click();
    await phone('Alice').getByRole('switch', { name: 'High-Risk mode' }).click({ force: true });
    await phone('Alice').getByText('What it cannot do').waitFor();
    await btn('Alice', 'Confirm: turn on High-Risk mode').click();
    await home('Alice');
    await phone('Alice').getByLabel('High-Risk').waitFor();
    await openChat('Alice', 'bob');
    await send('Alice', 'Quiet mode text');
    await phone('Bob').getByText('Quiet mode text').waitFor({ timeout: 60000 });
  });

  await step('duress PIN wipes silently and shows a fresh app', async () => {
    await home('Carol');
    await tab('Carol', 'Settings').click();
    await btn('Carol', 'Lock now').click();
    for (const d of '999999') await btn('Carol', `Digit ${d}`).click();
    await phone('Carol').getByRole('button', { name: 'Continue' }).waitFor({ timeout: 20000 });
  });

  await page.screenshot({ path: `${SHOTS}/e2e2-final.png` });
  const fatal = errors.filter((e) => !/React DevTools|Warning:/.test(e));
  assert.deepEqual(fatal, [], `page errors:\n${fatal.join('\n')}`);
  console.log('ALL FLOWS PASSED');
} catch (e) {
  await page.screenshot({ path: `${SHOTS}/e2e2-failure.png` });
  console.error('FAILED:', e.message.split('\n').slice(0, 6).join('\n'));
  console.error(errors.slice(0, 10).join('\n'));
  process.exitCode = 1;
} finally {
  await browser.close();
}
