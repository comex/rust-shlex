#![no_main]
#[macro_use] extern crate libfuzzer_sys;
use shlex::try_quote;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = try_quote(s);
    }
});
