#![no_main]
#[macro_use] extern crate libfuzzer_sys;
use shlex::try_join;
use nu_pretty_hex::pretty_hex;

use pyo3::prelude::*;

fn shlex_split(words: &str) -> Result<Vec<String>, String> {
    Python::with_gil(|py| {
        Ok(py
            .import("shlex").unwrap()
            .getattr("split").unwrap()
            .call1((words,))
            .map_err(|e| e.to_string())?
            .extract().unwrap())
    })
}

fn pretty_hex_multi<'a>(strings: impl IntoIterator<Item = &'a str>) -> String {
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
    let Ok(unquoted) = std::str::from_utf8(unquoted) else {
        // ignore invalid utf-8
        return;
    };
    let words: Vec<&str> = unquoted.split('\0').collect();
    let quoted: String = try_join(words.iter().cloned()).unwrap();
    let res = shlex_split(&quoted);

    match res {
        Ok(expanded) => {
            if expanded != words {
                panic!("original: {}\nshlex.split output:{}\nquoted:\n{}",
                       pretty_hex_multi(words.iter().cloned()),
                       pretty_hex_multi(expanded.iter().map(|x| &**x)),
                       pretty_hex(&quoted));
            }
        }
        Err(err) => {
            panic!("original: {}\nquoted:\n{}\nshlex.split error: {}",
                   pretty_hex_multi(words.iter().cloned()),
                   pretty_hex(&quoted),
                   err);
        },
    }
});

