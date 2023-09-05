// Copyright 2015 Nicholas Allegra (comex).
// Licensed under the Apache License, Version 2.0 <https://www.apache.org/licenses/LICENSE-2.0> or
// the MIT license <https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

//! [`Shlex`] and friends for byte strings.
//!
//! This is used internally by the [outer module](crate), and may be more
//! convenient if you are working with byte slices (`[u8]`) or types that are
//! wrappers around bytes, such as [`OsStr`](std::ffi::OsStr):
//!
//! ```rust
//! #[cfg(unix)] {
//!     use shlex::bytes::quote;
//!     use std::ffi::OsStr;
//!     use std::os::unix::ffi::OsStrExt;
//!
//!     // `\x80` is invalid in UTF-8.
//!     let os_str = OsStr::from_bytes(b"a\x80b c");
//!     assert_eq!(quote(os_str.as_bytes()), &b"\"a\x80b c\""[..]);
//! }
//! ```
//!
//! (On Windows, `OsStr` uses 16 bit wide characters so this will not work.)

extern crate alloc;
use alloc::vec::Vec;
use alloc::borrow::Cow;
#[cfg(test)]
use alloc::vec;
#[cfg(test)]
use alloc::borrow::ToOwned;

/// An iterator that takes an input byte string and splits it into the words using the same syntax as
/// the POSIX shell.
pub struct Shlex<'a> {
    in_iter: core::slice::Iter<'a, u8>,
    /// The number of newlines read so far, plus one.
    pub line_no: usize,
    /// An input string is erroneous if it ends while inside a quotation or right after an
    /// unescaped backslash.  Since Iterator does not have a mechanism to return an error, if that
    /// happens, Shlex just throws out the last token, ends the iteration, and sets 'had_error' to
    /// true; best to check it after you're done iterating.
    pub had_error: bool,
}

impl<'a> Shlex<'a> {
    pub fn new(in_bytes: &'a [u8]) -> Self {
        Shlex {
            in_iter: in_bytes.iter(),
            line_no: 1,
            had_error: false,
        }
    }

    fn parse_word(&mut self, mut ch: u8) -> Option<Vec<u8>> {
        let mut result: Vec<u8> = Vec::new();
        loop {
            match ch as char {
                '"' => if let Err(()) = self.parse_double(&mut result) {
                    self.had_error = true;
                    return None;
                },
                '\'' => if let Err(()) = self.parse_single(&mut result) {
                    self.had_error = true;
                    return None;
                },
                '\\' => if let Some(ch2) = self.next_char() {
                    if ch2 != '\n' as u8 { result.push(ch2); }
                } else {
                    self.had_error = true;
                    return None;
                },
                ' ' | '\t' | '\n' => { break; },
                _ => { result.push(ch as u8); },
            }
            if let Some(ch2) = self.next_char() { ch = ch2; } else { break; }
        }
        Some(result)
    }

    fn parse_double(&mut self, result: &mut Vec<u8>) -> Result<(), ()> {
        loop {
            if let Some(ch2) = self.next_char() {
                match ch2 as char {
                    '\\' => {
                        if let Some(ch3) = self.next_char() {
                            match ch3 as char {
                                // \$ => $
                                '$' | '`' | '"' | '\\' => { result.push(ch3); },
                                // \<newline> => nothing
                                '\n' => {},
                                // \x => =x
                                _ => { result.push('\\' as u8); result.push(ch3); }
                            }
                        } else {
                            return Err(());
                        }
                    },
                    '"' => { return Ok(()); },
                    _ => { result.push(ch2); },
                }
            } else {
                return Err(());
            }
        }
    }

    fn parse_single(&mut self, result: &mut Vec<u8>) -> Result<(), ()> {
        loop {
            if let Some(ch2) = self.next_char() {
                match ch2 as char {
                    '\'' => { return Ok(()); },
                    _ => { result.push(ch2); },
                }
            } else {
                return Err(());
            }
        }
    }

    fn next_char(&mut self) -> Option<u8> {
        let res = self.in_iter.next().copied();
        if res == Some(b'\n') { self.line_no += 1; }
        res
    }
}

impl<'a> Iterator for Shlex<'a> {
    type Item = Vec<u8>;
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(mut ch) = self.next_char() {
            // skip initial whitespace
            loop {
                match ch as char {
                    ' ' | '\t' | '\n' => {},
                    '#' => {
                        while let Some(ch2) = self.next_char() {
                            if ch2 as char == '\n' { break; }
                        }
                    },
                    _ => { break; }
                }
                if let Some(ch2) = self.next_char() { ch = ch2; } else { return None; }
            }
            self.parse_word(ch)
        } else { // no initial character
            None
        }
    }

}

/// Convenience function that consumes the whole byte string at once.  Returns None if the input was
/// erroneous.
pub fn split(in_bytes: &[u8]) -> Option<Vec<Vec<u8>>> {
    let mut shl = Shlex::new(in_bytes);
    let res = shl.by_ref().collect();
    if shl.had_error { None } else { Some(res) }
}

/// Given a single word, return a byte string suitable to encode it as a shell argument.
///
/// If given valid UTF-8, this will never produce invalid UTF-8. This is because it only
/// ever inserts valid ASCII characters before or after existing ASCII characters (or
/// returns two double quotes if the input was an empty string). It will never modify a
/// multibyte UTF-8 character.
pub fn quote(in_bytes: &[u8]) -> Cow<[u8]> {
    if in_bytes.len() == 0 {
        b"\"\""[..].into()
    } else if in_bytes.iter().any(|c| match *c as char {
        '|' | '&' | ';' | '<' | '>' | '(' | ')' | '$' | '`' | '\\' | '"' | '\'' | ' ' | '\t' |
        '\r' | '\n' | '*' | '?' | '[' | '#' | '~' | '=' | '%' => true,
        _ => false
    }) {
        let mut out: Vec<u8> = Vec::new();
        out.push(b'"');
        for &c in in_bytes {
            match c {
                b'$' | b'`' | b'"' | b'\\' => out.push(b'\\'),
                _ => ()
            }
            out.push(c);
        }
        out.push(b'"');
        out.into()
    } else {
        in_bytes.into()
    }
}

/// Convenience function that consumes an iterable of words and turns it into a single byte string,
/// quoting words when necessary. Consecutive words will be separated by a single space.
pub fn join<'a, I: core::iter::IntoIterator<Item = &'a [u8]>>(words: I) -> Vec<u8> {
    words.into_iter()
        .map(quote)
        .collect::<Vec<_>>()
        .join(&b' ')
}

#[cfg(test)]
const INVALID_UTF8: &[u8] = b"\xa1";

#[test]
fn test_invalid_utf8() {
    // Check that our test string is actually invalid UTF-8.
    assert!(core::str::from_utf8(INVALID_UTF8).is_err());
}

#[cfg(test)]
static SPLIT_TEST_ITEMS: &'static [(&'static [u8], Option<&'static [&'static [u8]]>)] = &[
    (b"foo$baz", Some(&[b"foo$baz"])),
    (b"foo baz", Some(&[b"foo", b"baz"])),
    (b"foo\"bar\"baz", Some(&[b"foobarbaz"])),
    (b"foo \"bar\"baz", Some(&[b"foo", b"barbaz"])),
    (b"   foo \nbar", Some(&[b"foo", b"bar"])),
    (b"foo\\\nbar", Some(&[b"foobar"])),
    (b"\"foo\\\nbar\"", Some(&[b"foobar"])),
    (b"'baz\\$b'", Some(&[b"baz\\$b"])),
    (b"'baz\\\''", None),
    (b"\\", None),
    (b"\"\\", None),
    (b"'\\", None),
    (b"\"", None),
    (b"'", None),
    (b"foo #bar\nbaz", Some(&[b"foo", b"baz"])),
    (b"foo #bar", Some(&[b"foo"])),
    (b"foo#bar", Some(&[b"foo#bar"])),
    (b"foo\"#bar", None),
    (b"'\\n'", Some(&[b"\\n"])),
    (b"'\\\\n'", Some(&[b"\\\\n"])),
    (INVALID_UTF8, Some(&[INVALID_UTF8])),
];

#[test]
fn test_split() {
    for &(input, output) in SPLIT_TEST_ITEMS {
        assert_eq!(split(input), output.map(|o| o.iter().map(|&x| x.to_owned()).collect()));
    }
}

#[test]
fn test_lineno() {
    let mut sh = Shlex::new(b"\nfoo\nbar");
    while let Some(word) = sh.next() {
        if word == b"bar" {
            assert_eq!(sh.line_no, 3);
        }
    }
}

#[test]
fn test_quote() {
    assert_eq!(quote(b"foobar"), &b"foobar"[..]);
    assert_eq!(quote(b"foo bar"), &b"\"foo bar\""[..]);
    assert_eq!(quote(b"\""), &b"\"\\\"\""[..]);
    assert_eq!(quote(b""), &b"\"\""[..]);
    assert_eq!(quote(INVALID_UTF8), INVALID_UTF8);
}

#[test]
fn test_join() {
    assert_eq!(join(vec![]), &b""[..]);
    assert_eq!(join(vec![&b""[..]]), &b"\"\""[..]);
    assert_eq!(join(vec![&b"a"[..], &b"b"[..]]), &b"a b"[..]);
    assert_eq!(join(vec![&b"foo bar"[..], &b"baz"[..]]), &b"\"foo bar\" baz"[..]);
    assert_eq!(join(vec![INVALID_UTF8]), INVALID_UTF8);
}
