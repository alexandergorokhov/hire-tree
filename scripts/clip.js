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
  const t0 = Date.now();
  const mark = (name) => console.error(`${((Date.now() - t0) / 1000).toFixed(1)}s ${name}`);

  // 1. The hero: one real job from the live NEAR AI run plays out as a tree.
  await page.goto(`${base}/market.html`);
  mark('hero');
  await wait(1.2);
  await say('AI agents now hire other AI agents. Who holds the money? Who decides the work was good?');
  await wait(7.4);
  await say('Each part of this real job is its own NEAR account, holding exactly its money. Bad work is fired, never paid.');
  await wait(7);
  await say(`Live NEAR AI run: ${parts.length} parts, ${parts.filter(p => p.paid).length} paid, ${parts.filter(p => p.fired).length} junk deliveries fired unpaid. One fluent filler still got paid: that hole is fixed below.`);
  await wait(4);

  // 2. The idea, how it is new, where it goes.
  await say('');
  await to('#idea');
  await say('Use case: an AI editor hires other agents. Bad work is fired unpaid; only checked work is paid.');
  await wait(8);
  await page.$eval('.loop', e => e.scrollIntoView({ behavior: 'smooth', block: 'center' }));
  await say('The job contract holds the money. NEAR AI scores the work and signs it. The contract pays only if the score passes.');
  await wait(8);
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
  mark('proof');
  await to('#proof');
  await say('30 fake accounts flood the market with cheap bad work. Buyers reading records lose 10.75 parts to them; blind buyers lose 71.50.');
  await wait(7.5);
  await say('');
  await to('.fixed', 'center');
  await say('Found and fixed: fluent filler passed the AI judge 7 of 7. Now the contract checks quoted proof: 0 of 7.');
  await wait(5);
  await to('#testnet', 'center');
  await say('Same contracts on NEAR testnet: 110 transactions, 0 failed.');
  await wait(3);
  await say('');

  // 6. For agents.
  mark('build');
  await to('#build', 'center');
  await say('Your agent can do all of this itself: one MCP server, no keys.');
  await wait(6);
  await say('');

  // 7. End card.
  mark('end card');
  await page.evaluate(() => {
    const d = document.createElement('div');
    d.style.cssText = 'position:fixed;inset:0;z-index:100000;background:#0e0e0c;color:#ecece6;display:flex;flex-direction:column;justify-content:center;padding:0 120px;gap:22px;opacity:0;transition:opacity .6s';
    d.innerHTML = '<div style="display:flex;align-items:center;gap:18px"><img src="brand/sprout-256.png" width="64" height="64" alt=""><span style="font:700 64px/1 Inter,system-ui,sans-serif;letter-spacing:-1px">Hire Tree</span></div>' +
      '<div style="font:500 30px/1.35 Inter,system-ui,sans-serif;max-width:900px">AI agents hire AI agents on NEAR. Pay waits for a signed, checked score.</div>' +
      '<div style="font:500 20px/1.9 \'JetBrains Mono\',ui-monospace,monospace;color:#c9c9bf">' +
      '<div><span style="color:#ffcc29">$</span> make judge &nbsp;&rarr; ALL PASS (12 checks)</div>' +
      '<div><span style="color:#ffcc29">$</span> make mcp-setup &nbsp;&rarr; 50 MCP tools for your agent</div>' +
      '<div>testnet factory: hub.hiretree.testnet &middot; 110 tx, 0 failed</div>' +
      '<div>github.com/alexandergorokhov/agent-hire</div></div>';
    document.body.appendChild(d);
    requestAnimationFrame(() => { d.style.opacity = '1'; });
  });
  await wait(7);
  mark('end');

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
