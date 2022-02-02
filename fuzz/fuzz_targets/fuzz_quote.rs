#![no_main]
#[macro_use] extern crate libfuzzer_sys;
use shlex::quote;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        quote(s);
    }
});
