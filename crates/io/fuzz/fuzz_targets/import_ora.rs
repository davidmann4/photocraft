#![no_main]
//! Import arbitrary bytes as OpenRaster .ora (ZIP + stack.xml) through the same entry point File › Open uses.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = photocraft_io::import("fuzz.ora", data);
});
