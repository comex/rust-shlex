/* SPDX-License-Identifier: MIT OR Apache-2.0 */

//! On Windows, an `OsStr` is a byte-encoded string with 16-bit characters. Transcoding is
//! required so this API is more iterator-based.

use core::iter;
use std::ffi::{OsStr, OsString};

use alloc::borrow::Cow;

use crate::bytes::{self, strategy_from_flags, DOUBLE_QUOTED_OK, SINGLE_QUOTED_OK, UNQUOTED_OK};
use crate::bytes::{check_char_quoting, QuotingStrategy};
use crate::QuoteError;

use std::os::windows::ffi::{EncodeWide, OsStrExt, OsStringExt};

type OsIter<'a> = EncodeWide<'a>;

const NUL: u16 = '\0' as u16;
const SPACE: u16 = ' ' as u16;
const DOLLAR: u16 = '$' as u16;
const HASH: u16 = '#' as u16;
const NEWLINE: u16 = '\n' as u16;
const TAB: u16 = '\t' as u16;
const DOUBLE_QUOTE: u16 = '"' as u16;
const SINGLE_QUOTE: u16 = '\'' as u16;
const BACKSLASH: u16 = '\\' as u16;
const BACKTICK: u16 = '`' as u16;

/// An iterator that takes an input byte string and splits it into the words using the same syntax as
/// the POSIX shell.
pub struct Shlex<'a> {
    in_iter: EncodeWide<'a>,
    /// The number of newlines read so far, plus one.
    pub line_no: usize,
    /// An input string is erroneous if it ends while inside a quotation or right after an
    /// unescaped backslash.  Since Iterator does not have a mechanism to return an error, if that
    /// happens, Shlex just throws out the last token, ends the iteration, and sets 'had_error' to
    /// true; best to check it after you're done iterating.
    had_error: bool,
}

impl<'a> Shlex<'a> {
    fn parse_word(&mut self, mut ch: u16) -> Option<OsString> {
        let mut result: Vec<u16> = Vec::new();
        loop {
            match ch {
                DOUBLE_QUOTE => {
                    if let Err(()) = self.parse_double(&mut result) {
                        self.had_error = true;
                        return None;
                    }
                }
                SINGLE_QUOTE => {
                    if let Err(()) = self.parse_single(&mut result) {
                        self.had_error = true;
                        return None;
                    }
                }
                BACKSLASH => {
                    if let Some(ch2) = self.next_char() {
                        if ch2 != NEWLINE {
                            result.push(ch2);
                        }
                    } else {
                        self.had_error = true;
                        return None;
                    }
                }
                SPACE | TAB | NEWLINE => break,
                _ => result.push(ch),
            }
            if let Some(ch2) = self.next_char() {
                ch = ch2;
            } else {
                break;
            }
        }
        Some(OsString::from_wide(&result))
    }

    fn parse_double(&mut self, result: &mut Vec<u16>) -> Result<(), ()> {
        loop {
            if let Some(ch2) = self.next_char() {
                match ch2 {
                    BACKSLASH => {
                        if let Some(ch3) = self.next_char() {
                            match ch3 {
                                // \$ => $
                                DOLLAR | BACKTICK | DOUBLE_QUOTE | BACKSLASH => result.push(ch3),
                                // \<newline> => nothing
                                NEWLINE => {}
                                // \x => =x
                                _ => {
                                    result.push(b'\\'.into());
                                    result.push(ch3);
                                }
                            }
                        } else {
                            return Err(());
                        }
                    }
                    DOUBLE_QUOTE => return Ok(()),
                    _ => result.push(ch2),
                }
            } else {
                return Err(());
            }
        }
    }

    fn parse_single(&mut self, result: &mut Vec<u16>) -> Result<(), ()> {
        loop {
            if let Some(ch2) = self.next_char() {
                match ch2 {
                    SINGLE_QUOTE => return Ok(()),
                    _ => result.push(ch2),
                }
            } else {
                return Err(());
            }
        }
    }

    fn next_char(&mut self) -> Option<u16> {
        let res = self.in_iter.next();
        if res == Some(NEWLINE) {
            self.line_no += 1;
        }
        res
    }
}

impl Iterator for Shlex<'_> {
    type Item = OsString;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(mut ch) = self.next_char() {
            // skip initial whitespace
            loop {
                match ch {
                    SPACE | TAB | NEWLINE => {}
                    HASH => {
                        while let Some(ch2) = self.next_char() {
                            if ch2 == NEWLINE {
                                break;
                            }
                        }
                    }
                    _ => break,
                }
                if let Some(ch2) = self.next_char() {
                    ch = ch2;
                } else {
                    return None;
                }
            }
            self.parse_word(ch)
        } else {
            // no initial character
            None
        }
    }
}

pub fn shlex_new<'a>(in_str: &'a OsStr) -> Shlex<'a> {
    Shlex {
        in_iter: in_str.encode_wide(),
        line_no: 1,
        had_error: false,
    }
}

pub fn shlex_next(this: &mut Shlex) -> Option<OsString> {
    this.next()
}

pub fn split(in_str: &OsStr) -> Option<Vec<OsString>> {
    let mut shl = shlex_new(in_str);
    let res = shl.by_ref().collect();
    if shl.had_error {
        None
    } else {
        Some(res)
    }
}

pub fn join<'a, I: IntoIterator<Item = &'a OsStr>>(
    quoter: &bytes::Quoter,
    words: I,
) -> Result<OsString, QuoteError> {
    let quoted_words = words.into_iter().map(|word| quote(quoter, word));

    let mut ret = OsString::new();
    let mut first = true;
    for word in quoted_words {
        if !first {
            ret.push(" ");
        }
        first = true;
        ret.push(word?);
    }

    Ok(ret)
}

pub fn quote<'a>(quoter: &bytes::Quoter, in_str: &'a OsStr) -> Result<Cow<'a, OsStr>, QuoteError> {
    if in_str.is_empty() {
        // Empty string.  Special case that isn't meaningful as only part of a word.
        return Ok(OsStr::new("''").into());
    }

    if !quoter.allow_nul && in_str.encode_wide().any(|ch| ch == NUL) {
        return Err(QuoteError::Nul);
    }

    let mut iter = in_str.encode_wide();
    let mut out: Vec<u16> = Vec::new();

    while iter.clone().next().is_some() {
        // Pick a quoting strategy for some prefix of the input.  Normally this will cover the
        // entire input, but in some case we might need to divide the input into multiple chunks
        // that are quoted differently.
        let (cur_len, strategy) = quoting_strategy(iter.clone());
        if cur_len == in_str.len() && strategy == QuotingStrategy::Unquoted && out.is_empty() {
            // Entire string can be represented unquoted.  Reuse the allocation.
            return Ok(in_str.into());
        }

        append_quoted_chunk(&mut out, (&mut iter).take(cur_len), strategy);
    }

    Ok(OsString::from_wide(&out).into())
}

#[cfg_attr(manual_codegen_check, inline(never))]
fn quoting_strategy(mut iter: EncodeWide) -> (usize, QuotingStrategy) {
    let mut prev_ok = SINGLE_QUOTED_OK | DOUBLE_QUOTED_OK | UNQUOTED_OK;

    let mut i = 0;

    while let Some(c) = iter.next() {
        if i == 0 && c == u16::from(b'^') {
            // To work around a Bash bug, ^ is only allowed right after an opening single quote; see
            // quoting_warning.
            prev_ok = SINGLE_QUOTED_OK;
            i += 1;
            continue;
        }

        let mut cur_ok = prev_ok;

        if c >= 0x80 {
            // Normally, non-ASCII characters shouldn't require quoting, but see quoting_warning.md
            // about \xa0.  For now, just treat all non-ASCII characters as requiring quotes.  This
            // also ensures things are safe in the off-chance that you're in a legacy 8-bit locale that
            // has additional characters satisfying `isblank`.
            cur_ok &= !UNQUOTED_OK;
        } else {
            check_char_quoting(c as u8, &mut cur_ok);
        }

        if cur_ok == 0 {
            // There are no quoting strategies that would work for both the previous characters and
            // this one.  So we have to end the chunk before this character.  The caller will call
            // `quoting_strategy` again to handle the rest of the string.
            break;
        }

        prev_ok = cur_ok;
        i += 1;
    }

    let strategy = strategy_from_flags(prev_ok);
    debug_assert!(i > 0);
    (i, strategy)
}

fn append_quoted_chunk(
    out: &mut Vec<u16>,
    iter: iter::Take<&mut OsIter>,
    strategy: QuotingStrategy,
) {
    match strategy {
        QuotingStrategy::Unquoted => out.extend(iter),
        QuotingStrategy::SingleQuoted => {
            let (lower, upper) = iter.size_hint();
            out.reserve(upper.unwrap_or(lower) + 2);
            out.push(b'\''.into());
            out.extend(iter);
            out.push(b'\''.into());
        }
        QuotingStrategy::DoubleQuoted => {
            let (lower, upper) = iter.size_hint();
            out.reserve(upper.unwrap_or(lower) + 2);
            out.push(b'"'.into());
            for c in iter {
                if let DOLLAR | BACKTICK | DOUBLE_QUOTE | BACKSLASH = c {
                    // Add a preceding backslash.
                    // Note: We shouldn't actually get here for $ and ` because they don't pass
                    // `double_quoted_ok`.
                    out.push(b'\\'.into());
                }
                // Add the character itself.
                out.push(c);
            }
            out.push(b'"'.into());
        }
    }
}

#[cfg(test)]
pub fn extra_split_tests() -> Vec<(OsString, Vec<OsString>)> {
    let invalid_utf8 = OsString::from_wide(&[bytes::INVALID_UTF8[0].into()]);

    // From the `String::from_utf16` examples
    let good16 = &[0xD834, 0xDD1E, 0x006d, 0x0075, 0x0073, 0x0069, 0x0063];
    let bad16 = &[0xD834, 0xDD1E, 0x006d, 0x0075, 0xD800, 0x0069, 0x0063];
    assert!(String::from_utf16(good16).is_ok());
    assert!(String::from_utf16(bad16).is_err());
    let valid_utf16 = OsString::from_wide(good16);
    let invalid_utf16 = OsString::from_wide(bad16);

    vec![
        (invalid_utf8.clone(), vec![invalid_utf8]),
        (invalid_utf16.clone(), vec![invalid_utf16]),
        (valid_utf16.clone(), vec![valid_utf16]),
    ]
}

#[cfg(test)]
pub fn extra_quote_tests() -> Vec<(OsString, OsString)> {
    let mut ret = Vec::new();
    let mut buf = Vec::new();

    // Just make sure things still work with UTF-16
    for (unquoted, quoted_expected) in crate::test_cases() {
        buf.clear();
        buf.extend(unquoted.encode_utf16());
        let unquoted = OsString::from_wide(&buf);
        buf.clear();
        buf.extend(quoted_expected.encode_utf16());
        let quoted_expected = OsString::from_wide(&buf);
        ret.push((unquoted, quoted_expected));
    }

    ret
}
