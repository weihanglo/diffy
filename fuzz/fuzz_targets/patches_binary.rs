#![no_main]

use diffy::patches::{ParseOptions, Patches};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &str| {
    // Fuzz binary patch parsing through Patches::parse with keep_binary()
    // This exercises base85 decoding, zlib decompression, and delta application
    let _: Result<Vec<_>, _> = Patches::parse(data, ParseOptions::gitdiff().keep_binary()).collect();
});
