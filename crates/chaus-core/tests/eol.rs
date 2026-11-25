//! Newline detection, normalization and reconstruction.

use chaus_core::eol::{Eol, EolMap};

#[test]
fn normalizes_and_records_each_break_kind() {
    let (text, map) = EolMap::from_source("a\r\nb\nc\rd");
    assert_eq!(text, "a\nb\nc\nd");
    assert_eq!(map.kinds(), vec![Eol::CrLf, Eol::Lf, Eol::Cr]);
}

#[test]
fn round_trips_mixed_endings() {
    let source = "one\r\ntwo\nthree\rfour";
    let (text, map) = EolMap::from_source(source);
    assert_eq!(map.apply_to(&text, Eol::Lf), source);
}

#[test]
fn splices_preserve_untouched_breaks() {
    let mut map = EolMap::new(vec![Eol::Lf, Eol::CrLf, Eol::Lf]);
    map.splice(1, 1, [Eol::Cr, Eol::Lf]);
    assert_eq!(map.kinds(), vec![Eol::Lf, Eol::Cr, Eol::Lf, Eol::Lf]);
}

#[test]
fn detects_dominant_kind() {
    assert_eq!(Eol::detect("a\r\nb\r\nc"), Eol::CrLf);
    assert_eq!(Eol::detect("a\nb\nc"), Eol::Lf);
    assert_eq!(Eol::detect("a\rb\rc"), Eol::Cr);
    assert_eq!(Eol::detect("no breaks"), Eol::Lf);
}
