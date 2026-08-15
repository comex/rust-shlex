/* SPDX-License-Identifier: MIT OR Apache-2.0 */

//! On Unix and WASI an `OsStr` is just a u8 slice, so we can use `bytes`.
// FIXME: The WASI traits are currently not stable. Add support when available.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::{OsStrExt, OsStringExt};

use alloc::borrow::Cow;

use crate::bytes;
use crate::QuoteError;

pub type Shlex<'a> = bytes::Shlex<'a>;

pub fn shlex_new<'a>(in_str: &'a OsStr) -> Shlex<'a> {
    bytes::Shlex::new(in_str.as_bytes())
}

pub fn shlex_next(this: &mut bytes::Shlex) -> Option<OsString> {
    this.next().map(OsString::from_vec)
}

pub fn split(in_str: &OsStr) -> Option<Vec<OsString>> {
    let mut shl = Shlex::new(in_str.as_bytes());
    let res = shl.by_ref().map(OsString::from_vec).collect();
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
    let words = words.into_iter().map(OsStr::as_bytes);
    quoter.join(words).map(OsString::from_vec)
}

pub fn quote<'a>(quoter: &bytes::Quoter, in_str: &'a OsStr) -> Result<Cow<'a, OsStr>, QuoteError> {
    match quoter.quote(in_str.as_bytes())? {
        Cow::Borrowed(out) => Ok(OsStr::from_bytes(out).into()),
        Cow::Owned(out) => Ok(OsString::from_vec(out).into()),
    }
}

#[cfg(test)]
pub fn extra_split_tests() -> Vec<(OsString, Vec<OsString>)> {
    let invalid_utf8 = OsStr::from_bytes(bytes::INVALID_UTF8).to_owned();
    vec![(invalid_utf8.clone(), vec![invalid_utf8])]
}

#[cfg(test)]
pub fn extra_quote_tests() -> Vec<(OsString, OsString)> {
    // This mostly exists for Windows
    Vec::new()
}
