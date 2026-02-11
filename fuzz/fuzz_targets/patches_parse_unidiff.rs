#![no_main]

use diffy::patches::{ParseOptions, Patches};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &str| {
    // Should never panic - only return Ok or Err
    let _: Result<Vec<_>, _> = Patches::parse(data, ParseOptions::unidiff()).collect();
});
