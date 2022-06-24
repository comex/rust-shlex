#![no_main]
#[macro_use] extern crate libfuzzer_sys;
use shlex::Shlex;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let mut sh = Shlex::new(s);
        while let Some(word) = sh.next() {}
    }
});
