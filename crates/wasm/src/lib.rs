//! The engine as a WebAssembly module with a plain C ABI, so a page can use
//! it with nothing but `WebAssembly.instantiate`.
//!
//! The host copies a resolved fixture's JSON into memory from [`alloc`],
//! calls [`run`], and reads the result: a little-endian `u32` length, a
//! status byte (0 for a trace, 1 for an error message), then that many bytes
//! of UTF-8. It hands both buffers back with [`dealloc`].

use tickwright_trace::ResolvedFixture;

/// Allocate `len` bytes for the host to write into.
#[unsafe(no_mangle)]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::<u8>::with_capacity(len);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// Free a buffer from [`alloc`] or [`run`].
///
/// # Safety
/// `ptr` and `len` must describe a buffer this module handed out.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: usize) {
    drop(unsafe { Vec::from_raw_parts(ptr, 0, len) });
}

/// Run the resolved fixture in `ptr[..len]` with the rules of `version`
/// (`version_ptr[..version_len]`) and return the result buffer.
///
/// # Safety
/// Both ranges must be initialised memory inside this module's heap.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn run(ptr: *const u8, len: usize, version_ptr: *const u8, version_len: usize) -> *mut u8 {
    let input = unsafe { std::slice::from_raw_parts(ptr, len) };
    let version = unsafe { std::slice::from_raw_parts(version_ptr, version_len) };
    let (status, body) = match simulate(input, version) {
        Ok(json) => (0u8, json),
        Err(e) => (1u8, format!("{e:#}")),
    };
    let mut out = Vec::with_capacity(5 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.push(status);
    out.extend_from_slice(body.as_bytes());
    let ptr = out.as_mut_ptr();
    std::mem::forget(out);
    ptr
}

/// Run the resolved fixture without recording, for benchmarks. Returns 0 on
/// success and 1 if the fixture could not run.
///
/// # Safety
/// As for [`run`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bench(ptr: *const u8, len: usize, version_ptr: *const u8, version_len: usize) -> u32 {
    let input = unsafe { std::slice::from_raw_parts(ptr, len) };
    let version = unsafe { std::slice::from_raw_parts(version_ptr, version_len) };
    let go = || -> anyhow::Result<()> {
        let fixture = serde_json::from_str::<ResolvedFixture>(std::str::from_utf8(input)?)?;
        tickwright_engine::run::run_unrecorded(&fixture, std::str::from_utf8(version)?)
    };
    u32::from(go().is_err())
}

fn simulate(input: &[u8], version: &[u8]) -> anyhow::Result<String> {
    let fixture = serde_json::from_str::<ResolvedFixture>(std::str::from_utf8(input)?)?;
    let trace = tickwright_engine::run::run(&fixture, std::str::from_utf8(version)?)?;
    Ok(serde_json::to_string(&trace)? + "\n")
}
