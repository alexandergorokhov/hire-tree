// Record a silent demo clip of the product page (web/market.html) as a
// person uses it, with short captions burned in.
//   node scripts/clip.js   (or make clip)
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
      el.style.cssText = 'position:fixed;left:50%;bottom:28px;transform:translateX(-50%);max-width:980px;width:calc(100% - 64px);' +
        'background:#0e0e0c;color:#ecece6;font:500 16px/1.5 "JetBrains Mono",ui-monospace,monospace;padding:12px 18px;border-radius:2px;border-left:6px solid #ffcc29;z-index:99999;text-align:left';
      document.body.appendChild(el);
    }
    el.textContent = t;
    el.style.display = t ? 'block' : 'none';
  }, text);
}

(async () => {
  const live = JSON.parse(fs.readFileSync(path.join(web, 'swarm-live.json')));
  const parts = live.rounds.flatMap(r => r.parts);
  fs.rmSync(outDir, { recursive: true, force: true });
  fs.mkdirSync(outDir, { recursive: true });
  const srv = await serve();
  const base = `http://127.0.0.1:${srv.address().port}`;
  const browser = await chromium.launch();
  const ctx = await browser.newContext({ viewport: { width: W, height: H }, recordVideo: { dir: outDir, size: { width: W, height: H } } });
  const page = await ctx.newPage();
  const wait = (s) => page.waitForTimeout(s * 1000);
  const to = async (sel, block = 'start') => {
    await page.$eval(sel, (e, b) => e.scrollIntoView({ behavior: 'smooth', block: b }), block);
    await wait(1.2);
  };
  const say = (t) => caption(page, t);

  // 1. The hero: one real job from the live NEAR AI run plays out as a tree.
  await page.goto(`${base}/market.html`);
  await wait(15.6);
  await say(`Live NEAR AI run: ${parts.length} parts, ${parts.filter(p => p.paid).length} paid, ${parts.filter(p => p.fired).length} bad deliveries fired with no pay.`);
  await wait(4);

  // 2. The idea, how it is new, where it goes.
  await say('');
  await to('#idea');
  await say('The job contract holds the money. NEAR AI scores the work and signs it. The contract pays only if the score passes.');
  await wait(9);
  await page.$eval('.new', e => e.scrollIntoView({ behavior: 'smooth', block: 'center' }));
  await say('What is new: one escrow per job, pay decided by a verified AI score, agents hiring agents.');
  await wait(6);

  // 3. The catalog.
  await say('');
  await to('#agents');
  await say('Agents for hire, ranked by their on-chain record.');
  await wait(3.5);
  await page.hover('.agent:nth-child(2)');
  await wait(1);
  await page.click('.agent:nth-child(2)');
  await say('Every job, judge score and payment. Hire it from your own agent over MCP.');
  await wait(6);
  await page.keyboard.press('Escape');
  await say('');
  await page.click('#skills .chip:nth-child(2)');
  await wait(2);
  const junk = await page.$('.agent:last-child');
  if (junk) await junk.evaluate(e => e.scrollIntoView({ behavior: 'smooth', block: 'center' }));
  await say('Ill-intended agents bid low, get fired unpaid, and their record loses the next award.');
  await wait(5);
  await page.click('#skills .chip:nth-child(1)');
  await say('');

  // 4. Jobs: open the one with a sub-hire and a rehire.
  await to('#jobs');
  const jobs = await page.$$('.job > button');
  const pick = Math.max(0, live.rounds.flatMap(r => r.plans.map((_, i) => r.intents[i] && r.intents[i].root))
    .findIndex(root => parts.some(p => p.root === root && p.level === 2)));
  await jobs[pick].click();
  await wait(0.6);
  await to(`.job:nth-child(${pick + 2})`, 'center');
  await say('A contractor hired its own sub-agent from its own pay. Junk was fired and replaced.');
  await wait(7);
  await say('');

  // 5. Proof under attack.
  await to('#proof');
  await say('30 fake accounts flood the market with cheap bad work. Buyers reading records lose 10.75 parts to them; blind buyers lose 71.50.');
  await wait(7.5);
  await say('');

  // 6. For agents.
  await to('#build', 'center');
  await say('Your agent can do all of this itself: one MCP server, no keys.');
  await wait(5.5);
  await say('');

  // 7. The 12-round simulation, same page.
  await to('#top');
  await page.click('#run-sim');
  await wait(1);
  await to('#agents');
  await say('Switch to the 12-round market: the same records, more history.');
  await wait(5);
  await say('');
  await to('#top');
  await wait(2);

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
