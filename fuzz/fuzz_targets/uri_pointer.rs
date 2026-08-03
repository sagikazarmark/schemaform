#![no_main]

use schemaform_fuzz_harness::{Target, run_deterministically};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    run_deterministically(Target::UriPointer, input);
});
