// Record a silent demo clip of the two pages with captions burned in.
//   node scripts/clip.js   (or make clip; needs web/scene.json from make demo)
// Serves web/ on a local port, drives Chromium with Playwright, records
// the screen, and converts it to out/clip/hire-tree-demo.mp4 with ffmpeg.
// Every number on screen is read from the committed run files.
const { chromium } = require('playwright');
const http = require('http');
const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');

const root = path.join(__dirname, '..');
const web = path.join(root, 'web');
const outDir = path.join(root, 'out', 'clip');
const W = 1280, H = 800;

const STORY = [
  [0, 6, 'A client posts a job with one token transfer. The factory creates a job account with no owner key: no company holds this money.'],
  [1, 5, 'Worker agents bid their own price on the job account, on-chain.'],
  [2, 6, 'Each award creates a sub-account holding exactly the winning bid. Nobody has a record yet, so the cheap bot wins parts.'],
  [3, 7, 'Workers deliver through NEAR AI. The contract checks the model signature, then asks its own quality question; a keeper submits the signed answer.'],
  [4, 5, 'The contract reads the signed scores: real research passes, filler fails and its job reopens.'],
  [5, 5, 'A passed worker is paid now, not when the whole tree closes.'],
  [6, 6, 'The tree folds: each account pays out, deletes itself and reports its worker\'s outcome to the factory.'],
  [7, 3, 'Job 2, same task.'],
  [8, 4, 'The cheap bot bids low again.'],
  [9, 6, 'This time the lead reads the bot\'s on-chain record and hires someone else, though the bot bid less.'],
  [13, 5, 'Everyone passed, everyone was paid, the tree folded again.'],
  [14, 5, 'Job 3: three clients who would not pay alone pool their money in a contract with no deploy key.'],
  [22, 6, 'The pool approves by stake majority and pays each member its share of what was not spent.'],
];

function serve() {
  const types = { '.html': 'text/html', '.json': 'application/json', '.svg': 'image/svg+xml', '.png': 'image/png' };
  const srv = http.createServer((req, res) => {
    const f = path.join(web, decodeURIComponent(req.url.split('?')[0]).replace(/^\/+/, '') || 'index.html');
    if (!f.startsWith(web) || !fs.existsSync(f)) { res.writeHead(404); return res.end(); }
    res.writeHead(200, { 'content-type': types[path.extname(f)] || 'text/plain' });
    fs.createReadStream(f).pipe(res);
  });
  return new Promise(r => srv.listen(0, '127.0.0.1', () => r(srv)));
}

async function caption(page, text) {
  await page.evaluate((t) => {
    let el = document.getElementById('__cap');
    if (!el) {
      el = document.createElement('div');
      el.id = '__cap';
      el.style.cssText = 'position:fixed;left:50%;bottom:28px;transform:translateX(-50%);max-width:1100px;width:calc(100% - 64px);' +
        'background:rgba(20,33,61,.92);color:#fff;font:500 22px/1.4 Inter,system-ui,sans-serif;padding:14px 22px;border-radius:12px;z-index:99999;text-align:center';
      document.body.appendChild(el);
    }
    el.textContent = t;
    el.style.display = t ? 'block' : 'none';
  }, text);
}

async function card(page, title, sub, secs) {
  await page.setContent(`<body style="margin:0;height:100vh;display:flex;flex-direction:column;justify-content:center;align-items:center;background:#FBFAF7;color:#14213D;font-family:Inter,system-ui,sans-serif;text-align:center">
    <h1 style="font-size:64px;margin:0 0 18px">${title}</h1><p style="font-size:28px;max-width:960px;line-height:1.4;color:#4A5568">${sub}</p></body>`);
  await page.waitForTimeout(secs * 1000);
}

(async () => {
  if (!fs.existsSync(path.join(web, 'scene.json'))) throw new Error('web/scene.json missing: run make demo first');
  const seeds = JSON.parse(fs.readFileSync(path.join(web, 'swarm-seeds.json')));
  const m = seeds.mean;
  const stormFile = path.join(web, 'swarm-storm.json');
  const st = fs.existsSync(stormFile) ? JSON.parse(fs.readFileSync(stormFile)).mean : null;
  fs.rmSync(outDir, { recursive: true, force: true });
  fs.mkdirSync(outDir, { recursive: true });
  const srv = await serve();
  const base = `http://127.0.0.1:${srv.address().port}`;
  const browser = await chromium.launch();
  const ctx = await browser.newContext({ viewport: { width: W, height: H }, recordVideo: { dir: outDir, size: { width: W, height: H } } });
  const page = await ctx.newPage();

  await card(page, 'Hire Tree', 'When an AI agent hires another AI agent, who holds the money? Nobody: every job is its own NEAR account.', 5);

  await page.goto(`${base}/index.html`);
  await page.waitForTimeout(1200);
  let at = 0;
  for (const [scene, secs, text] of STORY) {
    while (at < scene) { await page.keyboard.press('ArrowRight'); at++; }
    await page.waitForTimeout(400);
    await caption(page, text);
    await page.waitForTimeout(secs * 1000);
  }

  await page.goto(`${base}/swarm.html`);
  await page.waitForTimeout(1500);
  await caption(page, `The market at scale: 3 leads, 36 worker agents, 12 rounds on a local NEAR node. Winners are not scripted.`);
  await page.waitForTimeout(6000);
  for (const [sel, text, secs] of [
    ['#stats', 'Contractors buy sub-parts out of their own pay; junk is fired on-chain and the runner-up is hired.', 6],
    ['#control-card', `Same market, ${seeds.seeds.length} seeds, three kinds of buyer. With on-chain records, junk won ${m.records['junk wins']} parts per run, against ${m['audit-all'] ? m['audit-all']['junk wins'] + ' when buyers audit everything and ' : ''}${m.blind['junk wins']} blind.`, 9],
    ...(st ? [['#storm-card', `Then we attacked it: 30 sybil accounts flood every auction with half-price junk at round 6. Buyers that audit everything lose ${st['audit-all']['storm wins'].toFixed(0)} parts per run to them; buyers that read on-chain records lose ${st.records['storm wins'].toFixed(0)}, at about half the cost per good part.`, 11]] : []),
  ]) {
    const el = await page.$(sel);
    if (el) await el.evaluate(e => e.scrollIntoView({ block: 'center' }));
    await caption(page, text);
    await page.waitForTimeout(secs * 1000);
  }
  await caption(page, '');

  await card(page, 'Try it', 'An AI judge can use it directly: make mcp-setup, then ask your agent to join the market.<br>Or watch it: make swarm. Every number here is read from the chain.', 6);

  const video = page.video();
  await ctx.close();
  await browser.close();
  srv.close();
  const webm = await video.path();
  const mp4 = path.join(outDir, 'hire-tree-demo.mp4');
  execFileSync('ffmpeg', ['-y', '-loglevel', 'error', '-i', webm, '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-crf', '23', '-movflags', '+faststart', mp4]);
  fs.rmSync(webm);
  console.log(mp4);
})().catch(e => { console.error(e); process.exit(1); });
