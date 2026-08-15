/* SPDX-License-Identifier: MIT OR Apache-2.0 */

//! [`Shlex`] and friends for [`OsStr`].
//!
//! This can be used to split or join an `OsStr` directly. Unlike [`bytes`], the extension traits
//! don't need to be used directly, and this is supported on Windows as well.
//!
//! Note that while this works on Windows, it still uses a POSIX syntax (like other `shlex`
//! modules). This is likely sufficient for tasks like splitting environment variables, but
//! results may not be appropriate for passing to Windows shells directly.
//!
//! This module is only available on platforms that have an `OsStrExt` and `OsStringExt`: currently
//! only Unix and Windows.
//!
//! ```rust
//! #[cfg(unix)] {
//!     use shlex::bytes::try_quote;
//!     use std::ffi::OsStr;
//!     use std::os::unix::ffi::OsStrExt;
//!
//!     // `\x80` is invalid in UTF-8.
//!     let os_str = OsStr::from_bytes(b"a\x80b c");
//!     assert_eq!(try_quote(os_str.as_bytes()).unwrap(), &b"'a\x80b c'"[..]);
//! }
//!
//! #[cfg(windows)] {
//!     use shlex::os_str::try_quote;
//!     use std::ffi::OsString;
//!     use std::os::windows::ffi::OsStringExt;
//!
//!     // Wide char constructor
//!     fn w(ch: u8) -> u16 { ch.into() }
//!
//!     // `\x80` is invalid in UTF-16.
//!     let os_str = OsString::from_wide(&[w(b'a'), w(0x80), w(b'b'), w(b' '), w(b'c')]);
//!     let expected = OsString::from_wide(
//!         &[w(b'\''), w(b'a'), w(0x80), w(b'b'), w(b' '), w(b'c'), w(b'\'')]
//!     );
//!     assert_eq!(try_quote(&os_str).unwrap(), expected);
//! }
//! ```
//!
//! [`OsStr`]: std::ffi::OsStr

#[cfg(unix)]
#[path = "bytestr.rs"]
mod imp;
#[cfg(windows)]
#[path = "widestr.rs"]
mod imp;

use std::ffi::{OsStr, OsString};

use crate::bytes;
#[cfg(all(doc, not(doctest)))]
use crate::{self as shlex, quoting_warning};
use alloc::borrow::Cow;
use alloc::vec::Vec;

use super::QuoteError;

/// An iterator that takes an input byte string and splits it into the words using the same syntax
/// as the POSIX shell.
pub struct Shlex<'a>(imp::Shlex<'a>);

impl<'a> Shlex<'a> {
    pub fn new(in_str: &'a OsStr) -> Self {
        Self(imp::shlex_new(in_str))
    }
}

impl Iterator for Shlex<'_> {
    type Item = OsString;
    fn next(&mut self) -> Option<Self::Item> {
        imp::shlex_next(&mut self.0)
    }
}

/// Convenience function that consumes the whole byte string at once.  Returns None if the input was
/// erroneous.
pub fn split(in_str: &OsStr) -> Option<Vec<OsString>> {
    imp::split(in_str)
}

/// A more configurable interface to quote strings.  If you only want the default settings you can
/// use the convenience functions [`try_quote`] and [`try_join`].
///
/// The string equivalent is [`shlex::Quoter`].
#[derive(Default, Debug, Clone)]
pub struct Quoter {
    inner: bytes::Quoter,
}

impl Quoter {
    /// Create a new [`Quoter`] with default settings.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set whether to allow [nul bytes](quoting_warning#nul-bytes).  By default they are not
    /// allowed and will result in an error of [`QuoteError::Nul`].
    #[inline]
    pub fn allow_nul(mut self, allow: bool) -> Self {
        self.inner = self.inner.allow_nul(allow);
        self
    }

    /// Convenience function that consumes an iterable of words and turns it into a single byte
    /// string, quoting words when necessary. Consecutive words will be separated by a single
    /// space.
    pub fn join<'a, I: IntoIterator<Item = &'a OsStr>>(
        &self,
        words: I,
    ) -> Result<OsString, QuoteError> {
        imp::join(&self.inner, words)
    }

    /// Given a single word, return a byte string suitable to encode it as a shell argument.
    ///
    /// If given a valid `OsStr`, this will never produce an invalid `OsStr`; see
    /// [`bytes::Quoter::quote`] for details.
    pub fn quote<'a>(&self, in_str: &'a OsStr) -> Result<Cow<'a, OsStr>, QuoteError> {
        imp::quote(&self.inner, in_str)
    }
}

/// Convenience function that consumes an iterable of words and turns it into a single `OsString`,
/// quoting words when necessary. Consecutive words will be separated by a single space.
///
/// Uses default settings. The only error that can be returned is [`QuoteError::Nul`].
///
/// Equivalent to [`Quoter::new().join(words)`](Quoter).
///
/// The string equivalent is [shlex::try_join].
pub fn try_join<'a, I: IntoIterator<Item = &'a OsStr>>(words: I) -> Result<OsString, QuoteError> {
    Quoter::new().join(words)
}

/// Given a single word, return a string suitable to encode it as a shell argument.
///
/// Uses default settings. The only error that can be returned is [`QuoteError::Nul`].
///
/// Equivalent to [`Quoter::new().quote(in_bytes)`](Quoter).
///
/// The string equivalent is [shlex::try_quote].
pub fn try_quote(in_str: &OsStr) -> Result<Cow<'_, OsStr>, QuoteError> {
    Quoter::new().quote(in_str)
}

#[test]
fn test_split() {
    for &(input, output) in crate::SPLIT_TEST_ITEMS {
        let input = OsStr::new(input);
        assert_eq!(
            split(input),
            output.map(|o| o.iter().map(|&x| OsString::from(x)).collect())
        );
    }

    for (input, output) in imp::extra_split_tests() {
        assert_eq!(split(&input).unwrap(), output);
    }
}

#[test]
fn test_lineno() {
    let mut sh = Shlex::new(OsStr::new("\nfoo\nbar"));
    while let Some(word) = sh.next() {
        if word == "bar" {
            assert_eq!(sh.0.line_no, 3);
        }
    }
}

#[test]
fn test_quote() {
    let mut ok = true;
    for (unquoted, quoted_expected) in crate::test_cases() {
        let unquoted = OsStr::new(&unquoted);
        let quoted_expected = OsStr::new(&quoted_expected);
        let quoted_actual = try_quote(&unquoted).unwrap();
        if quoted_expected != quoted_actual {
            println!(
                "FAIL: for input <{}>, expected <{}>, got <{}>",
                unquoted.to_string_lossy(),
                quoted_expected.to_string_lossy(),
                quoted_actual.to_string_lossy()
            );
            ok = false;
        }
    }
    for (unquoted, quoted_expected) in imp::extra_quote_tests() {
        let quoted_actual = try_quote(&unquoted).unwrap();
        if quoted_expected != quoted_actual {
            println!(
                "FAIL: for input <{}>, expected <{}>, got <{}>",
                unquoted.to_string_lossy(),
                quoted_expected.to_string_lossy(),
                quoted_actual.to_string_lossy()
            );
            ok = false;
        }
    }
    assert!(ok);
}

#[test]
fn test_fallible() {
    assert_eq!(try_join(vec![OsStr::new("\0")]), Err(QuoteError::Nul));
    assert_eq!(try_quote(OsStr::new("\0")), Err(QuoteError::Nul));
}
