#![no_main]
//! Import arbitrary bytes as GIMP .xcf through the same entry point File › Open uses. The
//! `.xcf.gz` name lets gzip input reach the bounded inflate; raw XCF is still found by its magic.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // A raster or PSD signature wins over the name and goes to a decoder with targets of its own
    // (crates/codecs/fuzz, crates/psd/fuzz); spending time there only finds big legal canvases.
    if photocraft_codecs::detect(data).is_some() || data.starts_with(b"8BPS") {
        return;
    }
    let _ = photocraft_io::import("fuzz.xcf.gz", data);
});
