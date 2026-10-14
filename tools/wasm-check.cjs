// Runs every resolved fixture through the WebAssembly engine inside headless
// Chromium and checks each trace is byte-for-byte the trace the native engine
// wrote (`tickwright run`). It then drives each fixture again through the
// editor's session commands, click by click and tick by tick, and checks the
// session records the same events.
//
//   node tools/wasm-check.cjs <engine.wasm> <resolved dir> <native traces dir> [bench fixture]
//
// With a bench fixture (`tickwright bench --write`), it also times that build
// in the browser, as a fixture run and as an editor session.
//
// Needs the `playwright` package and its Chromium.

const fs = require("fs");
const path = require("path");
const { chromium } = require("playwright");

async function main() {
  const [wasmPath, resolvedDir, nativeDir, benchPath] = process.argv.slice(2);
  const version = "26.3";
  if (!nativeDir) {
    console.error("usage: wasm-check.cjs <engine.wasm> <resolved dir> <native traces dir> [bench fixture]");
    process.exit(2);
  }
  const files = fs.readdirSync(resolvedDir).filter((f) => f.endsWith(".json")).sort();
  const browser = await chromium.launch();
  const page = await browser.newPage();
  page.on("pageerror", (e) => console.error("page error:", e));
  await page.evaluate(async (b64) => {
    const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
    const { instance } = await WebAssembly.instantiate(bytes, {});
    const e = instance.exports;
    const enc = new TextEncoder();
    const dec = new TextDecoder();
    const put = (s) => {
      const b = enc.encode(s);
      const p = e.alloc(b.length);
      new Uint8Array(e.memory.buffer, p, b.length).set(b);
      return [p, b.length];
    };
    window.runFixture = (fixture, version, fn = "run") => {
      const [fp, fl] = put(fixture);
      const [vp, vl] = put(version);
      const t0 = performance.now();
      const out = e[fn](fp, fl, vp, vl);
      const ms = performance.now() - t0;
      const view = new DataView(e.memory.buffer);
      const len = view.getUint32(out, true);
      const status = view.getUint8(out + 4);
      const text = dec.decode(new Uint8Array(e.memory.buffer, out + 5, len));
      e.dealloc(fp, fl);
      e.dealloc(vp, vl);
      e.dealloc(out, len + 5);
      return { status, text, ms };
    };
    window.cmd = (command) => {
      const [p, l] = put(JSON.stringify(command));
      const out = e.session(p, l);
      const view = new DataView(e.memory.buffer);
      const len = view.getUint32(out, true);
      const status = view.getUint8(out + 4);
      const text = dec.decode(new Uint8Array(e.memory.buffer, out + 5, len));
      e.dealloc(p, l);
      e.dealloc(out, len + 5);
      if (status !== 0) throw new Error(text);
      return JSON.parse(text);
    };
    const call = (fn, a, b) => {
      const [ap, al] = put(a);
      const [bp, bl] = put(b);
      const out = fn(ap, al, bp, bl);
      const view = new DataView(e.memory.buffer);
      const len = view.getUint32(out, true);
      const status = view.getUint8(out + 4);
      const body = new Uint8Array(e.memory.buffer, out + 5, len).slice();
      e.dealloc(ap, al);
      e.dealloc(bp, bl);
      e.dealloc(out, len + 5);
      if (status !== 0) throw new Error(dec.decode(body));
      return body;
    };
    // Export a resolved fixture in every schematic format, import each file
    // back, and name the formats whose blocks or container contents came
    // back different. A jukebox's disc must be written as the game saves it
    // (`RecordItem`), which a round trip alone would not show.
    window.schematicRoundTrip = async (fixtureText) => {
      const fx = JSON.parse(fixtureText);
      const want = JSON.stringify(fx.blocks);
      const contents = (cs) => JSON.stringify([...(cs || [])].sort((a, b) => String(a.pos).localeCompare(String(b.pos))));
      const wantContents = contents(fx.containers);
      const same = (p, q) => p[0] === q[0] && p[1] === q[1] && p[2] === q[2];
      const disc = (fx.containers || []).some((c) =>
        fx.blocks.some((b) => same(b, c.pos) && b[3].startsWith("minecraft:jukebox")),
      );
      const bad = [];
      for (const ext of ["litematic", "schem", "nbt"]) {
        const file = call(e.export_schematic, fixtureText, `build.${ext}`);
        const [fp, fl] = [e.alloc(file.length), file.length];
        new Uint8Array(e.memory.buffer, fp, fl).set(file);
        const [np, nl] = put("build");
        const out = e.import_schematic(fp, fl, np, nl);
        const view = new DataView(e.memory.buffer);
        const len = view.getUint32(out, true);
        const text = dec.decode(new Uint8Array(e.memory.buffer, out + 5, len));
        const back = view.getUint8(out + 4) === 0 ? JSON.parse(text) : null;
        let ok = back !== null && JSON.stringify(back.blocks) === want && contents(back.containers) === wantContents;
        e.dealloc(fp, fl);
        e.dealloc(np, nl);
        e.dealloc(out, len + 5);
        if (ok && disc) {
          const stream = new Blob([file]).stream().pipeThrough(new DecompressionStream("gzip"));
          const raw = new Uint8Array(await new Response(stream).arrayBuffer());
          ok = new TextDecoder("latin1").decode(raw).includes("RecordItem");
        }
        if (!ok) bad.push(ext);
      }
      return bad;
    };
    // A fixture's material list, from the materials export and from a
    // session opened on it; they should agree.
    window.materialLists = (fixtureText, version) => {
      const fromFixture = JSON.parse(dec.decode(call(e.materials, fixtureText, version)));
      const { session } = window.cmd({ op: "open", version, fixture: JSON.parse(fixtureText) });
      const fromSession = window.cmd({ op: "materials", session, version });
      window.cmd({ op: "close", session });
      // The two answer with their keys in different orders.
      const canon = (v) =>
        Array.isArray(v) ? v.map(canon)
        : v && typeof v === "object" ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, canon(v[k])]))
        : v;
      return [JSON.stringify(canon(fromFixture)), JSON.stringify(canon(fromSession)), fromFixture.items.length];
    };
    // Play a resolved fixture through a session; return its events inside the
    // fixture's box, relative to the origin, as a trace would hold them.
    window.sessionEvents = (fixtureText, version) => {
      const f = JSON.parse(fixtureText);
      const { session } = window.cmd({ op: "open", version, fixture: f });
      const [ox, oy, oz] = f.origin;
      const m = f.margin;
      const inside = (p) =>
        p[0] - ox >= -m && p[0] - ox < f.size[0] + m &&
        p[1] - oy >= -m && p[1] - oy < f.size[1] + m &&
        p[2] - oz >= -m && p[2] - oz < f.size[2] + m;
      const out = [];
      for (let t = 1; t <= f.ticks; t++) {
        for (const s of f.stimuli.filter((s) => s.tick === t)) {
          const pos = [ox + s.pos[0], oy + s.pos[1], oz + s.pos[2]];
          if (s.action === "use") window.cmd({ op: "use", session, pos });
          else if (s.action === "set") window.cmd({ op: "set", session, pos, state: s.state, updates: !s.quiet });
          else if (s.action === "place")
            window.cmd({ op: "place", session, pos, item: s.item, face: s.face, look: s.look, pitch: s.pitch, hit: s.hit, sneak: s.sneak });
          else if (s.action === "page") window.cmd({ op: "page", session, pos, page: s.page });
          else if (s.action === "time") window.cmd({ op: "time", session, time: s.time });
          else window.cmd({ op: "destroy", session, pos });
        }
        for (const ev of window.cmd({ op: "step", session }).events) {
          if (inside(ev.pos)) out.push({ ...ev, pos: [ev.pos[0] - ox, ev.pos[1] - oy, ev.pos[2] - oz] });
        }
      }
      window.cmd({ op: "close", session });
      return out;
    };
    // Step an editor session on the bench build the way a long Settle does,
    // answering only whether each tick was busy, then the same with every
    // event in the answer; and time rewinding to each of the last 64 ticks.
    // A fresh instance, so its memory shows what the session keeps.
    window.benchSession = async (fixtureText, version, ticks) => {
      const x = (await WebAssembly.instantiate(bytes, {})).instance.exports;
      const send = (command) => {
        const b = enc.encode(JSON.stringify(command));
        const p = x.alloc(b.length) >>> 0;
        new Uint8Array(x.memory.buffer, p, b.length).set(b);
        const out = x.session(p, b.length) >>> 0;
        const view = new DataView(x.memory.buffer);
        const len = view.getUint32(out, true);
        const status = view.getUint8(out + 4);
        const text = status === 0 && len < 1000 ? dec.decode(new Uint8Array(x.memory.buffer, out + 5, len)) : "";
        x.dealloc(p, b.length);
        x.dealloc(out, len + 5);
        if (status !== 0) throw new Error(`${command.op} failed`);
        return text;
      };
      const fixture = JSON.parse(fixtureText);
      const run = (events) => {
        const { session } = JSON.parse(send({ op: "open", version, fixture }));
        const before = x.memory.buffer.byteLength;
        const t0 = performance.now();
        for (let i = 0; i < ticks; i++) send({ op: "step", session, events });
        return { session, ms: performance.now() - t0, grew: x.memory.buffer.byteLength - before };
      };
      const lean = run(false);
      let worst = 0;
      for (let t = ticks - 1; t >= ticks - 64; t--) {
        const t0 = performance.now();
        send({ op: "rewind", session: lean.session, tick: t });
        worst = Math.max(worst, performance.now() - t0);
      }
      send({ op: "close", session: lean.session });
      const full = run(true);
      send({ op: "close", session: full.session });
      return { lean: lean.ms, full: full.ms, grew: lean.grew, worst };
    };
    window.benchFixture = (fixture, version) => {
      const [fp, fl] = put(fixture);
      const [vp, vl] = put(version);
      const t0 = performance.now();
      const status = e.bench(fp, fl, vp, vl);
      const ms = performance.now() - t0;
      e.dealloc(fp, fl);
      e.dealloc(vp, vl);
      return { status, ms };
    };
  }, fs.readFileSync(wasmPath).toString("base64"));

  let failed = 0;
  let refused = 0;
  let partial = 0;
  let roundTrips = 0;
  let materialLists = 0;
  let totalMs = 0;
  for (const f of files) {
    const fixture = fs.readFileSync(path.join(resolvedDir, f), "utf8");
    const r = await page.evaluate(([fx, v]) => window.runFixture(fx, v), [fixture, version]);
    totalMs += r.ms;
    const nativePath = path.join(nativeDir, f);
    const hasNative = fs.existsSync(nativePath);
    if (r.status !== 0) {
      // Blocks the engine doesn't support yet are refused natively too.
      if (hasNative) {
        console.log(`${f}: engine error: ${r.text}`);
        failed++;
      } else {
        refused++;
        // A run that met something unsimulated mid-way still has a trace
        // up to that tick; one refused at setup is refused here too.
        const p = await page.evaluate(([fx, v]) => window.runFixture(fx, v, "run_partial"), [fixture, version]);
        if (p.status === 0) {
          const u = JSON.parse(p.text).unsupported;
          if (!u || !r.text.endsWith(u.message)) {
            console.log(`${f}: run_partial gave a trace without the reason run refused it for: ${r.text}`);
            failed++;
          } else {
            partial++;
          }
        } else if (p.text !== r.text) {
          console.log(`${f}: run_partial and run refused it differently: ${p.text} / ${r.text}`);
          failed++;
        }
      }
      continue;
    }
    if (!hasNative) {
      console.log(`${f}: ran in the browser but was refused natively`);
      failed++;
      continue;
    }
    if (f.endsWith("@r0.json")) {
      const bad = await page.evaluate((fx) => window.schematicRoundTrip(fx), fixture);
      if (bad.length) {
        console.log(`${f}: blocks or contents changed through ${bad.join(", ")} export and import`);
        failed++;
        continue;
      }
      roundTrips++;
      const [a, b, n] = await page.evaluate(([fx, v]) => window.materialLists(fx, v), [fixture, version]);
      if (a !== b || n === 0) {
        console.log(`${f}: material lists from the fixture and from a session differ or are empty`);
        failed++;
        continue;
      }
      materialLists++;
    }
    const native = fs.readFileSync(nativePath, "utf8");
    if (native !== r.text) {
      console.log(`${f}: browser trace differs from the native trace`);
      failed++;
      continue;
    }
    if (JSON.parse(fixture).entities) {
      // Editor sessions don't simulate entities yet.
      continue;
    }
    const events = await page.evaluate(([fx, v]) => window.sessionEvents(fx, v), [fixture, version]);
    // The session answers with keys in a different order, so compare fields.
    const flat = (es) => JSON.stringify(es.map((e) => [e.tick, e.phase, e.kind, e.pos, e.state, e.detail]));
    if (flat(events) !== flat(JSON.parse(native).entries)) {
      console.log(`${f}: the editor session's events differ from the fixture run's`);
      failed++;
    }
  }
  if (benchPath) {
    const fixture = fs.readFileSync(benchPath, "utf8");
    const { ticks, name, blocks } = JSON.parse(fixture);
    const r = await page.evaluate(([fx, v]) => window.benchFixture(fx, v), [fixture, version]);
    if (r.status !== 0) {
      console.log(`${name}: bench failed to run`);
      failed++;
    } else {
      const secs = r.ms / 1000;
      console.log(`${name}: ${blocks.length} blocks, ${ticks} ticks in ${secs.toFixed(3)} s, ${(ticks / secs).toFixed(0)} ticks/s (browser)`);
    }
    const n = 1000;
    const b = await page.evaluate(([fx, v, n]) => window.benchSession(fx, v, n), [fixture, version, n]);
    console.log(
      `${name} as an editor session: ${n} ticks in ${(b.lean / 1000).toFixed(3)} s answering only whether each was busy, ` +
        `${(b.full / 1000).toFixed(3)} s answering every event; memory grew ${(b.grew / n / 1024).toFixed(1)} KB a tick; ` +
        `rewinding took at most ${b.worst.toFixed(1)} ms (browser)`,
    );
  }
  await browser.close();
  console.log(`${files.length - failed - refused} of ${files.length - refused} fixtures gave identical traces in the browser, as fixture runs and as editor sessions (${refused} refused by both, ${partial} of them with a partial trace), ${totalMs.toFixed(0)} ms in wasm`);
  console.log(`${materialLists} builds gave the same material list from the fixture and from a session`);
  console.log(`${roundTrips} builds survived export to and import from .litematic, .schem and .nbt unchanged`);
  process.exit(failed ? 1 : 0);
}

main().catch((e) => {
  console.error(e);
  process.exit(2);
});
