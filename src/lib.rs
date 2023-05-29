// Copyright 2015 Nicholas Allegra (comex).
// Licensed under the Apache License, Version 2.0 <https://www.apache.org/licenses/LICENSE-2.0> or
// the MIT license <https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

//! Same idea as (but implementation not directly based on) the Python shlex module.  However, this
//! implementation does not support any of the Python module's customization because it makes
//! parsing slower and is fairly useless.  You only get the default settings of shlex.split, which
//! mimic the POSIX shell:
//! <https://pubs.opengroup.org/onlinepubs/9699919799/utilities/V3_chap02.html>
//!
//! This implementation also deviates from the Python version in not treating `\r` specially, which
//! I believe is more compliant.
//!
//! This is a string-friendly wrapper around the [bytes] module that works on the underlying byte
//! slices. The algorithms in this crate are oblivious to UTF-8 high bytes, so working directly
//! with bytes is a safe micro-optimization.
//!
//! Disabling the `std` feature (which is enabled by default) will allow the crate to work in
//! `no_std` environments, where the `alloc` crate, and a global allocator, are available.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;
use alloc::vec::Vec;
use alloc::borrow::Cow;
use alloc::string::String;
#[cfg(test)]
use alloc::vec;
#[cfg(test)]
use alloc::borrow::ToOwned;

pub mod bytes;

/// An iterator that takes an input string and splits it into the words using the same syntax as
/// the POSIX shell.
///
/// See [`bytes::Shlex`].
pub struct Shlex<'a>(bytes::Shlex<'a>);

impl<'a> Shlex<'a> {
    pub fn new(in_str: &'a str) -> Self {
        Self(bytes::Shlex::new(in_str.as_bytes()))
    }
}

impl<'a> Iterator for Shlex<'a> {
    type Item = String;
    fn next(&mut self) -> Option<String> {
        self.0.next().map(|byte_word| {
            // Safety: given valid UTF-8, bytes::Shlex will always return valid UTF-8.
            unsafe { String::from_utf8_unchecked(byte_word) }
        })
    }
}

impl<'a> core::ops::Deref for Shlex<'a> {
    type Target = bytes::Shlex<'a>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> core::ops::DerefMut for Shlex<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Convenience function that consumes the whole string at once.  Returns None if the input was
/// erroneous.
pub fn split(in_str: &str) -> Option<Vec<String>> {
    let mut shl = Shlex::new(in_str);
    let res = shl.by_ref().collect();
    if shl.had_error { None } else { Some(res) }
}

/// Given a single word, return a string suitable to encode it as a shell argument.
pub fn quote(in_str: &str) -> Cow<str> {
    match bytes::quote(in_str.as_bytes()) {
        Cow::Borrowed(out) => {
            // Safety: given valid UTF-8, bytes::quote() will always return valid UTF-8.
            unsafe { core::str::from_utf8_unchecked(out) }.into()
        }
        Cow::Owned(out) => {
            // Safety: given valid UTF-8, bytes::quote() will always return valid UTF-8.
            unsafe { String::from_utf8_unchecked(out) }.into()
        }
    }
}

/// Convenience function that consumes an iterable of words and turns it into a single string,
/// quoting words when necessary. Consecutive words will be separated by a single space.
pub fn join<'a, I: IntoIterator<Item = &'a str>>(words: I) -> String {
    words.into_iter()
        .map(quote)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
static SPLIT_TEST_ITEMS: &'static [(&'static str, Option<&'static [&'static str]>)] = &[
    ("foo$baz", Some(&["foo$baz"])),
    ("foo baz", Some(&["foo", "baz"])),
    ("foo\"bar\"baz", Some(&["foobarbaz"])),
    ("foo \"bar\"baz", Some(&["foo", "barbaz"])),
    ("   foo \nbar", Some(&["foo", "bar"])),
    ("foo\\\nbar", Some(&["foobar"])),
    ("\"foo\\\nbar\"", Some(&["foobar"])),
    ("'baz\\$b'", Some(&["baz\\$b"])),
    ("'baz\\\''", None),
    ("\\", None),
    ("\"\\", None),
    ("'\\", None),
    ("\"", None),
    ("'", None),
    ("foo #bar\nbaz", Some(&["foo", "baz"])),
    ("foo #bar", Some(&["foo"])),
    ("foo#bar", Some(&["foo#bar"])),
    ("foo\"#bar", None),
    ("'\\n'", Some(&["\\n"])),
    ("'\\\\n'", Some(&["\\\\n"])),
];

#[test]
fn test_split() {
    for &(input, output) in SPLIT_TEST_ITEMS {
        assert_eq!(split(input), output.map(|o| o.iter().map(|&x| x.to_owned()).collect()));
    }
}

#[test]
fn test_lineno() {
    let mut sh = Shlex::new("\nfoo\nbar");
    while let Some(word) = sh.next() {
        if word == "bar" {
            assert_eq!(sh.line_no, 3);
        }
    }
}

#[test]
fn test_quote() {
    assert_eq!(quote("foobar"), "foobar");
    assert_eq!(quote("foo bar"), "\"foo bar\"");
    assert_eq!(quote("\""), "\"\\\"\"");
    assert_eq!(quote(""), "\"\"");
}

#[test]
fn test_join() {
    assert_eq!(join(vec![]), "");
    assert_eq!(join(vec![""]), "\"\"");
    assert_eq!(join(vec!["a", "b"]), "a b");
    assert_eq!(join(vec!["foo bar", "baz"]), "\"foo bar\" baz");
}
