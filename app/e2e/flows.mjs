// End-to-end flows in the browser harness: real Rust core (WASM) in every
// phone, simulated Bluetooth between them, driven through the UI.
// Usage: node e2e/flows.mjs [baseUrl] [screenshotDir]
import { chromium } from '@playwright/test';
import assert from 'node:assert/strict';

const BASE = process.argv[2] ?? 'http://localhost:8088/';
const SHOTS = process.argv[3] ?? '.';
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1700, height: 1400 } });
const errors = [];
page.on('pageerror', (e) => errors.push(e.message));
page.on('console', (m) => m.type() === 'error' && errors.push(m.text()));

const phone = (name) => page.getByTestId(`phone-${name}`);
const step = async (label, fn) => {
  const t0 = Date.now();
  await fn();
  console.log(`✓ ${label} (${Date.now() - t0} ms)`);
};

async function onboard(name, pin = '123456') {
  const p = phone(name);
  await p.getByRole('button', { name: 'Skip' }).click();
  await p.getByLabel('Nickname').fill(name.toLowerCase());
  await p.getByLabel('App PIN (6 digits)').fill(pin);
  await p.getByLabel('Repeat PIN').fill(pin);
  await p.getByRole('button', { name: 'Create identity' }).click();
  await p.getByText('Chats', { exact: true }).waitFor({ timeout: 30000 });
}

async function openPair(name) {
  const p = phone(name);
  await p.getByRole('button', { name: 'New chat' }).click();
  await p.getByRole('button', { name: /Add contact in person/ }).click();
}

try {
  await page.goto(BASE);
  await phone('Bob').waitFor({ timeout: 60000 });

  await step('onboard Alice and Bob', async () => {
    await onboard('Alice');
    await onboard('Bob');
  });

  await step('phones discover each other and open an encrypted link', async () => {
    await page.waitForFunction(() => document.querySelectorAll('[data-testid="map"] line[stroke="#7cc4ff"]').length >= 1, null, { timeout: 20000 });
  });

  await step('in-person pairing through the simulated camera', async () => {
    await openPair('Bob'); // Show my code
    await phone('Bob').getByLabel('QR code').waitFor();
    await openPair('Alice');
    await phone('Alice').getByRole('radio', { name: 'Scan code' }).click();
    await phone('Alice').getByRole('radio', { name: "Bob's screen" }).click();
    await phone('Alice').getByText('Paired with bob').waitFor({ timeout: 30000 });
    await phone('Bob').getByText('Paired with alice').waitFor({ timeout: 30000 });
    assert.ok(await phone('Alice').getByLabel('Trust: Verified').first().isVisible());
  });

  await page.screenshot({ path: `${SHOTS}/e2e-paired.png` });

  await step('send a message and see it delivered and read', async () => {
    await phone('Bob').getByRole('button', { name: 'Done' }).click();
    await phone('Alice').getByRole('button', { name: 'Open chat' }).click();
    await phone('Alice').getByLabel('Message text').fill('Hello Bob, no internet here');
    await phone('Alice').getByRole('button', { name: 'Send', exact: true }).click();
    await phone('Bob').getByRole('button', { name: /alice/ }).first().click({ timeout: 20000 });
    await phone('Bob').getByText('Hello Bob, no internet here').waitFor({ timeout: 20000 });
    await phone('Alice').getByText('Read').waitFor({ timeout: 20000 });
  });

  await step('reply comes back', async () => {
    await phone('Bob').getByLabel('Message text').fill('Got it 👋');
    await phone('Bob').getByRole('button', { name: 'Send', exact: true }).click();
    await phone('Alice').getByText('Got it 👋').waitFor({ timeout: 20000 });
  });

  await step('safety numbers match on both phones', async () => {
    await phone('Alice').getByRole('button', { name: 'Conversation details' }).click();
    await phone('Bob').getByRole('button', { name: 'Conversation details' }).click();
    const a = await phone('Alice').getByLabel(/^Safety number \d/).getAttribute('aria-label');
    const b = await phone('Bob').getByLabel(/^Safety number \d/).getAttribute('aria-label');
    assert.equal(a, b);
    assert.match(a, /^Safety number (\d{5} ){5}\d{5}$/);
    await phone('Alice').getByRole('button', { name: 'Back', exact: true }).click();
    await phone('Bob').getByRole('button', { name: 'Back', exact: true }).click();
  });

  await step('relay: Carol carries a message when Alice and Bob are out of range', async () => {
    await page.getByRole('button', { name: '+ Phone', exact: true }).click();
    await phone('Carol').waitFor();
    await onboard('Carol');
    // Move Bob 54 m away (out of 30 m range) and Carol into the middle (drag on the map).
    const map = page.getByTestId('map');
    const box = await map.boundingBox();
    const drag = async (name, xm, ym) => {
      const dot = page.getByTestId(`map-phone-${name}`).locator('circle').first();
      const b = await dot.boundingBox();
      await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2);
      await page.mouse.down();
      await page.mouse.move(box.x + xm * 6, box.y + ym * 6, { steps: 8 });
      await page.mouse.up();
    };
    await drag('Bob', 84, 35);
    await drag('Carol', 57, 35);
    await page.waitForTimeout(2500);
    await phone('Alice').getByLabel('Message text').fill('Through Carol');
    await phone('Alice').getByRole('button', { name: 'Send', exact: true }).click();
    await phone('Bob').getByText('Through Carol').waitFor({ timeout: 30000 });
  });

  await step('delayed send holds until released', async () => {
    await phone('Alice').getByRole('button', { name: 'Send later' }).click();
    await phone('Alice').getByRole('radio', { name: 'Hold until I release' }).click();
    await phone('Alice').getByLabel('Message text').fill('Held back');
    await phone('Alice').getByRole('button', { name: 'Schedule' }).click();
    await phone('Alice').getByText('Held', { exact: true }).waitFor();
    await page.waitForTimeout(3000);
    assert.equal(await phone('Bob').getByText('Held back').count(), 0);
    await phone('Alice').getByRole('button', { name: 'Release now' }).click();
    await phone('Bob').getByText('Held back').waitFor({ timeout: 30000 });
  });

  await step('public channel post between strangers', async () => {
    for (const name of ['Carol', 'Bob']) {
      const p = phone(name);
      if (name === 'Bob') await p.getByRole('button', { name: 'Back', exact: true }).click();
      await p.getByRole('button', { name: 'New chat' }).click();
      await p.getByRole('button', { name: /Public channels/ }).click();
      await p.getByLabel('Channel name').fill('plaza');
      await p.getByRole('button', { name: 'Join' }).click();
    }
    await phone('Carol').getByLabel('Message text').fill('Anyone at the plaza?');
    await phone('Carol').getByRole('button', { name: 'Send', exact: true }).click();
    await phone('Bob').getByText('Anyone at the plaza?').waitFor({ timeout: 30000 });
  });

  await step('lock screen requires the PIN; wrong PIN is refused', async () => {
    const p = phone('Bob');
    while (!(await p.getByRole('tab', { name: 'Settings' }).isVisible())) {
      await p.getByRole('button', { name: 'Back', exact: true }).click();
    }
    await p.getByRole('tab', { name: 'Settings' }).click();
    await p.getByRole('button', { name: 'Lock now' }).click();
    await p.getByText('Meshtrike is locked').waitFor();
    for (const d of '000000') await p.getByRole('button', { name: `Digit ${d}` }).click();
    await p.getByText(/Wrong PIN/).waitFor({ timeout: 15000 });
    for (const d of '123456') await p.getByRole('button', { name: `Digit ${d}` }).click();
    await p.getByText('Chats', { exact: true }).waitFor({ timeout: 15000 });
  });

  await page.screenshot({ path: `${SHOTS}/e2e-final.png` });
  const fatal = errors.filter((e) => !/React DevTools|Warning:/.test(e));
  assert.deepEqual(fatal, [], `page errors:\n${fatal.join('\n')}`);
  console.log('ALL FLOWS PASSED');
} catch (e) {
  await page.screenshot({ path: `${SHOTS}/e2e-failure.png` });
  console.error('FAILED:', e.message);
  console.error(errors.slice(0, 10).join('\n'));
  process.exitCode = 1;
} finally {
  await browser.close();
}
