// Runs every resolved fixture through the WebAssembly engine inside headless
// Chromium and checks each trace is byte-for-byte the trace the native engine
// wrote (`tickwright run`).
//
//   node tools/wasm-check.cjs <engine.wasm> <resolved dir> <native traces dir> [bench fixture]
//
// With a bench fixture (`tickwright bench --write`), it also times that build
// in the browser.
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
    window.runFixture = (fixture, version) => {
      const [fp, fl] = put(fixture);
      const [vp, vl] = put(version);
      const t0 = performance.now();
      const out = e.run(fp, fl, vp, vl);
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
  let totalMs = 0;
  for (const f of files) {
    const fixture = fs.readFileSync(path.join(resolvedDir, f), "utf8");
    const r = await page.evaluate(([fx, v]) => window.runFixture(fx, v), [fixture, version]);
    totalMs += r.ms;
    if (r.status !== 0) {
      console.log(`${f}: engine error: ${r.text}`);
      failed++;
      continue;
    }
    const native = fs.readFileSync(path.join(nativeDir, f), "utf8");
    if (native !== r.text) {
      console.log(`${f}: browser trace differs from the native trace`);
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
  }
  await browser.close();
  console.log(`${files.length - failed} of ${files.length} fixtures gave identical traces in the browser (${totalMs.toFixed(0)} ms in wasm)`);
  process.exit(failed ? 1 : 0);
}

main().catch((e) => {
  console.error(e);
  process.exit(2);
});
