//! Strict parsing of the rc file's `nnoremap` subset.
//!
//! Only literal keys, `<leader>`, `<CR>`, `<Esc>`, `<Space>`, `<Tab>`, `<BS>` and
//! `<C-x>` are understood. Mappings are non-recursive and capped at eight keys so
//! an accidental cycle or an unbounded prefix cannot stall input.

use chaus_core::settings::RawMapping;

use crate::key::Key;

/// One validated Normal-mode mapping.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mapping {
    keys: Vec<Key>,
    rhs: Vec<Key>,
}

impl Mapping {
    #[must_use]
    pub fn keys(&self) -> &[Key] {
        &self.keys
    }

    #[must_use]
    pub fn rhs(&self) -> &[Key] {
        &self.rhs
    }
}

const MAX_KEYS: usize = 8;

/// Parse mappings, returning the valid subset and human-readable diagnostics.
#[must_use]
pub fn parse_mappings(raw: &[RawMapping], mapleader: char) -> (Vec<Mapping>, Vec<String>) {
    let mut mappings: Vec<Mapping> = Vec::new();
    let mut diagnostics = Vec::new();
    for entry in raw {
        let keys = match tokenize(&entry.lhs, mapleader) {
            Ok(keys) => keys,
            Err(message) => {
                diagnostics.push(format!("nnoremap {}: {message}", entry.lhs));
                continue;
            }
        };
        let rhs = match tokenize(&entry.rhs, mapleader) {
            Ok(keys) => keys,
            Err(message) => {
                diagnostics.push(format!("nnoremap {}: {message}", entry.lhs));
                continue;
            }
        };
        if mappings.iter().any(|mapping| mapping.keys == keys) {
            diagnostics.push(format!("nnoremap {}: duplicate mapping", entry.lhs));
            continue;
        }
        mappings.push(Mapping { keys, rhs });
    }
    (mappings, diagnostics)
}

fn tokenize(source: &str, mapleader: char) -> Result<Vec<Key>, String> {
    let mut keys = Vec::new();
    let mut characters = source.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '<' {
            let mut token = String::new();
            let mut closed = false;
            for character in characters.by_ref() {
                if character == '>' {
                    closed = true;
                    break;
                }
                token.push(character);
            }
            if !closed {
                return Err("unterminated <...> token".to_string());
            }
            if token.eq_ignore_ascii_case("leader") {
                keys.push(Key::Char(mapleader));
            } else if let Some(key) = Key::parse_token(&token) {
                keys.push(key);
            } else {
                return Err(format!("unsupported key token <{token}>"));
            }
        } else {
            keys.push(Key::Char(character));
        }
        if keys.len() > MAX_KEYS {
            return Err(format!("mapping is longer than {MAX_KEYS} keys"));
        }
    }
    if keys.is_empty() {
        return Err("mapping is empty".to_string());
    }
    Ok(keys)
}
