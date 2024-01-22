#![no_main]
#[macro_use] extern crate libfuzzer_sys;
use shlex::bytes::try_join;
use std::ptr;
use std::ffi::{c_char, CStr, CString};
use nu_pretty_hex::pretty_hex;

extern "C" {
    // wordexp_wrapper.c
    fn wordexp_wrapper(words: *const c_char, wordv_p: *mut *mut *mut c_char, wordc_p: *mut usize) -> *const c_char;
    fn wordfree_wrapper();
}

fn wordexp(words: Vec<u8>) -> Result<Vec<Vec<u8>>, String> {
    unsafe {
        let mut wordv: *mut *mut c_char = ptr::null_mut();
        let mut wordc: usize = 0;
        let cwords = CString::new(words).unwrap();
        let err = wordexp_wrapper(cwords.as_ptr(), &mut wordv, &mut wordc);
        if err.is_null() {
            // success
            let mut ret = Vec::new();
            for i in 0..wordc {
                ret.push(CStr::from_ptr(*wordv.add(i)).to_bytes().to_owned());
            }
            wordfree_wrapper();
            Ok(ret)
        } else {
            Err(CStr::from_ptr(err).to_string_lossy().to_string())
        }
    }
}

fn pretty_hex_multi<'a>(strings: impl IntoIterator<Item = &'a [u8]>) -> String {
    let mut res = "[\n".to_owned();
    for string in strings {
        res += &pretty_hex(&string);
        res.push('\n');
    }
    res.push(']');
    res
}

fuzz_target!(|unquoted: &[u8]| {
    // Treat the input as a list of words separated by nul chars.
    let words: Vec<&[u8]> = unquoted.split(|&c| c == b'\0').collect();
    let quoted: Vec<u8> = try_join(words.iter().cloned()).unwrap();

    let res = wordexp(quoted.clone());

    match res {
        Ok(expanded) => {
            if expanded != words {
                panic!("original: {}\nwordexp output:{}\nquoted:\n{}",
                       pretty_hex_multi(words.iter().cloned()),
                       pretty_hex_multi(expanded.iter().map(|x| &**x)),
                       pretty_hex(&quoted));
            }
        }
        Err(err) => {
            #[cfg(target_os = "macos")]
            if quoted.contains(&b'`') {
                // macOS wordexp bug
                return;
            }

            if err == "WRDE_NOSPACE" {
                // Input is probably too long.
                return;
            }

            panic!("original: {}\nquoted:\n{}\nwordexp error: {}",
                   pretty_hex_multi(words.iter().cloned()),
                   pretty_hex(&quoted),
                   err);
        },
    }
});

