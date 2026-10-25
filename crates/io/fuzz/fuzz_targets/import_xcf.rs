#![no_main]
//! Import arbitrary bytes as GIMP .xcf through the same entry point File › Open uses. The
//! `.xcf.gz` name lets gzip input reach the bounded inflate; raw XCF is still found by its magic.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = photocraft_io::import("fuzz.xcf.gz", data);
});
