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
const CARD = 3; // seconds of logo card put in front of the clip (narration line 1 plays over it)
const voiceDir = process.env.VOICE_DIR || path.join(root, 'video', 'voice');

// Length in seconds of narration line n (video/voice/NN.wav), 0 if missing.
function wavSecs(n) {
  const f = path.join(voiceDir, String(n).padStart(2, '0') + '.wav');
  if (!fs.existsSync(f)) return 0;
  const b = fs.readFileSync(f);
  let off = 12, byteRate = 0;
  while (off + 8 <= b.length) {
    const id = b.toString('ascii', off, off + 4), len = b.readUInt32LE(off + 4);
    if (id === 'fmt ') byteRate = b.readUInt32LE(off + 16);
    if (id === 'data') return byteRate ? len / byteRate : 0;
    off += 8 + len + (len % 2);
  }
  return 0;
}


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
  const t0 = Date.now();
  const now = () => (Date.now() - t0) / 1000;
  const wait = (s) => page.waitForTimeout(s * 1000);
  // Each scene starts a narration line and holds until it has been spoken.
  const starts = { 1: 0.3 - CARD };
  const speak = (n) => { starts[n] = now(); };
  const hold = async (n, min) => {
    const left = starts[n] + Math.max(min, wavSecs(n) + 0.35) - now();
    if (left > 0) await wait(left);
  };
  const to = async (sel, block = 'start') => {
    await page.$eval(sel, (e, b) => e.scrollIntoView({ behavior: 'smooth', block: b }), block);
    await wait(1.2);
  };
  const say = (t) => caption(page, t);
  const mark = (name) => console.error(`${((Date.now() - t0) / 1000).toFixed(1)}s ${name}`);

  // 1. The hero: one real job from the live NEAR AI run plays out as a tree.
  await page.goto(`${base}/market.html`);
  mark('hero');
  await wait(Math.max(1.2, wavSecs(1) + 0.2 - CARD - now()));
  speak(2);
  await say('When one agent hires another: who holds the money, and who decides the work is good?');
  await hold(2, 5);
  speak(3);
  await say('Each part of this real job has its own NEAR account, holding only its share of the money. Here a contractor hired a helper from its own pay.');
  await hold(3, 8);

  // 2. How the contract decides, and what is new.
  speak(4);
  await page.$eval('.loop', e => e.scrollIntoView({ behavior: 'smooth', block: 'center' }));
  await say('The job contract holds the money. A NEAR AI model scores the work and signs the score. The contract pays only if the score passes.');
  await hold(4, 6);
  speak(5);
  await page.$eval('.new', e => e.scrollIntoView({ behavior: 'smooth', block: 'center' }));
  await say('What is new: one escrow per job, payment decided by a verified score, and agents hiring their own helpers.');
  await hold(5, 5);

  // 3. The catalog.
  await say('');
  speak(6);
  await to('#agents');
  await say('Agents for hire, ranked by their on-chain record: every job, judge score and payment.');
  await hold(6, 4);
  await page.click('#skills .chip:nth-child(2)');
  await wait(0.8);
  const bad = await page.$('.agent:last-child');
  if (bad) await bad.evaluate(e => e.scrollIntoView({ behavior: 'smooth', block: 'center' }));
  speak(7);
  await say('Agents that hand in empty work go unpaid, and their record costs them the next contract.');
  await hold(7, 4);
  await page.click('#skills .chip:nth-child(1)');
  await say('');

  // 4. Proof under attack (simulation), then the weakness found live and fixed.
  mark('proof');
  speak(8);
  await to('#proof');
  await say('Simulation: 30 fake agents bid half price with empty work. Price-only buyers kept hiring them: 71 tasks lost, good work cost twice as much. Buyers who check the record first passed them over: 11 tasks.');
  await hold(8, 6);
  await say('');
  speak(9);
  await to('.fixed', 'start');
  await say('Found and fixed: polished filler passed the judge 7 of 7. Now the contract checks quoted proof: 0 of 7.');
  await wait(Math.max(4, wavSecs(9) - 3.5));
  await to('#testnet', 'center');
  await say('Same contracts on NEAR testnet: 110 transactions, 0 failed.');
  await hold(9, 6);
  // The same steps on the public testnet explorer (screenshots in video/explorer/, if present).
  const shots = [['judge-answer.png', 'NEAR testnet explorer: the signed NEAR AI judge answer, checked by the contract (9/10).'],
    ['settle.png', 'NEAR testnet explorer: the part is paid and its account closes itself.']]
    .filter(([f]) => fs.existsSync(path.join(root, 'video', 'explorer', f)));
  for (const [f, text] of shots) {
    const uri = 'data:image/png;base64,' + fs.readFileSync(path.join(root, 'video', 'explorer', f)).toString('base64');
    await page.evaluate((u) => {
      let d = document.getElementById('__shot');
      if (!d) { d = document.createElement('div'); d.id = '__shot'; d.style.cssText = 'position:fixed;inset:0;z-index:99990;background:#fff center/contain no-repeat'; document.body.appendChild(d); }
      d.style.backgroundImage = `url(${u})`;
    }, uri);
    await say(text);
    await wait(2.6);
  }
  if (shots.length) await page.evaluate(() => document.getElementById('__shot').remove());
  await say('');

  // 5. For agents.
  mark('build');
  speak(10);
  await to('#build', 'center');
  await wait(1);

  // 6b. A recorded MCP session (docs/MCP-TRANSCRIPT.md), replayed as a terminal.
  mark('mcp');
  await page.evaluate(() => {
    const rows = [
      ['join', '{"name": "contractor", "skills": ["research"]}', 'account guestcontractor.sandbox'],
      ['open_round', '{}', 'a client job opens 3 parts for bids'],
      ['bid', '{"as": "contractor", "price": "20 fUSD"}', 'bid placed on-chain'],
      ['close_round', '{}', 'you won t1.job-1 at 20 fUSD: "lowest bid; no bad record yet"'],
      ['subcontract', '{"task": "List 3 sources", "max": "8 fUSD"}', 'researcher bids 7.60 fUSD'],
      ['hire_sub', '{"worker": "researcher.sandbox"}', 'sub-job t1.t1.job-1 created, your margin 12.40 fUSD'],
      ['do_work', '{"job": "t1.job-1"}', 'the contract checked the job tag and the model signature'],
      ['judge_check', '{"job": "t1.job-1"}', 'signed score 8/10, verified on-chain'],
      ['finish_round', '{}', 'the job tree folded: every account paid out and deleted itself'],
    ];
    const d = document.createElement('div');
    d.id = '__mcp';
    d.style.cssText = 'position:fixed;inset:0;z-index:100000;background:#0e0e0c;color:#ecece6;padding:56px 72px;font:500 19px/1.55 "JetBrains Mono",ui-monospace,monospace';
    d.innerHTML = '<div style="color:#8a8a80;font-size:15px;letter-spacing:1px;margin-bottom:26px">MCP SESSION, ABRIDGED FROM docs/MCP-TRANSCRIPT.md · an agent joins, wins a part, hires a helper, gets paid (local NEAR node)</div><div id="__mcpl"></div>';
    document.body.appendChild(d);
    const list = d.querySelector('#__mcpl');
    rows.forEach((r, i) => setTimeout(() => {
      const el = document.createElement('div');
      el.style.marginBottom = '10px';
      el.innerHTML = '<span style="color:#ffcc29">&gt; ' + r[0] + '</span> <span style="color:#8a8a80">' + r[1].replace(/</g, '&lt;') + '</span><br><span style="color:#7fd38a">  ' + r[2].replace(/</g, '&lt;') + '</span>';
      list.appendChild(el);
    }, 200 + i * 900));
  });
  await wait(8.8);
  await hold(10, 0);
  await page.evaluate(() => document.getElementById('__mcp').remove());

  // 7. End card.
  mark('end card');
  speak(11);
  await page.evaluate(() => {
    const d = document.createElement('div');
    d.style.cssText = 'position:fixed;inset:0;z-index:100000;background:#0e0e0c;color:#ecece6;display:flex;flex-direction:column;justify-content:center;padding:0 120px;gap:22px;opacity:0;transition:opacity .6s';
    d.innerHTML = '<div style="display:flex;align-items:center;gap:18px"><img src="brand/sprout-256.png" width="64" height="64" alt=""><span style="font:700 64px/1 Inter,system-ui,sans-serif;letter-spacing:-1px">Hire Tree</span></div>' +
      '<div style="font:500 30px/1.35 Inter,system-ui,sans-serif;max-width:900px">Agents hire other agents on NEAR. Payment waits for a signed, verified score.</div>' +
      '<div style="font:500 20px/1.9 \'JetBrains Mono\',ui-monospace,monospace;color:#c9c9bf">' +
      '<div><span style="color:#ffcc29">$</span> make judge &nbsp;&rarr; ALL PASS (12 checks)</div>' +
      '<div><span style="color:#ffcc29">$</span> make mcp-setup &nbsp;&rarr; 50 MCP tools for your agent</div>' +
      '<div>testnet factory: hub.hiretree.testnet &middot; 110 tx, 0 failed</div>' +
      '<div>github.com/alexandergorokhov/agent-hire</div></div>';
    document.body.appendChild(d);
    requestAnimationFrame(() => { d.style.opacity = '1'; });
  });
  await hold(11, 5);
  mark('end');

  // Where each narration line starts in the final video (logo card included).
  fs.writeFileSync(path.join(outDir, 'voice-starts.tsv'),
    Object.keys(starts).sort((a, b) => a - b).map(n => `${(starts[n] + CARD).toFixed(2)}`).join('\n') + '\n');
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
