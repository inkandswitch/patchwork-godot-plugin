
use std::{fmt::Display, str::FromStr};

use serde::{Deserialize, Serialize};
use automerge::ChangeHash;
use samod::DocumentId;
// TODO (Lilith): Move this to utils
/// Represents a location anywhere in Patchwork's history.
/// Associates a branch with heads on that branch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryRef {
    /// The branch the ref is on.
    pub branch: DocumentId,
    // todo: it would be very nice to have a Heads struct
    /// The Automerge heads for the history location
    pub heads: Vec<ChangeHash>,
}

impl HistoryRef {
    pub const PATCHWORK_SCHEME_PREFIX: &'static str = "patchwork-";
    // these should be safe to use as path seperators; DocumentId is base58-encoded (only a-z, A-Z, 0-9), and ChangeHash is hex-encoded
    pub const BRANCH_DIVIDER: char = '+';
    pub const CHANGE_HASH_DIVIDER: char = '.';
    pub fn is_valid(&self) -> bool {
        return !self.heads.is_empty();
    }

    pub fn to_uri_scheme_prefix(&self) -> String {
        format!("{}{}", HistoryRef::PATCHWORK_SCHEME_PREFIX, self)
    }
}

impl Eq for HistoryRef {}

impl std::hash::Hash for HistoryRef {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.branch.hash(state);
        self.heads.hash(state);
    }
}

impl PartialEq for HistoryRef {
    fn eq(&self, other: &Self) -> bool {
        self.branch == other.branch && self.heads == other.heads
    }
}

impl Display for HistoryRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !self.is_valid() {
            return Err(std::fmt::Error);
        }
        let heads_str = self.heads.iter().map(|h| h.to_string()).collect::<Vec<String>>().join(&HistoryRef::CHANGE_HASH_DIVIDER.to_string());
        write!(f, "{}{}{}", self.branch, HistoryRef::BRANCH_DIVIDER, heads_str)
    }
}

// from str to history ref
impl FromStr for HistoryRef {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (doc_id, heads_part) = s
            .split_once(HistoryRef::BRANCH_DIVIDER)
            .ok_or_else(|| "Invalid history ref string")?;

        let branch = DocumentId::from_str(doc_id)
            .map_err(|_| "Invalid DocumentId in history ref string")?;

        let heads = if heads_part.is_empty() {
            Vec::new()
        } else {
            heads_part
                .split(HistoryRef::CHANGE_HASH_DIVIDER)
                .map(|h| ChangeHash::from_str(h)
                    .map_err(|_| "Invalid ChangeHash"))
                .collect::<Result<Vec<ChangeHash>, Self::Err>>()?
        };
        let result = HistoryRef { branch, heads };
        if !result.is_valid() {
            return Err("Invalid history ref");
        }
        Ok(result)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct HistoryRefPath {
    pub ref_: HistoryRef,
    pub path: String,
}

impl HistoryRefPath {
    pub const REF_DIVIDER: char = '-';

    pub fn recognize_path(path: &str) -> bool {
        HistoryRefPath::from_str(path).is_ok()
    }

    pub fn new(ref_: HistoryRef, path: String) -> Self {
        Self { ref_, path }
    }

    pub fn make_path_string(ref_: &HistoryRef, path: &str) -> Result<String, std::fmt::Error> {
        if !ref_.is_valid() {
            return Err(std::fmt::Error);
        }
        Ok(format!("{}{}{}", ref_.to_uri_scheme_prefix(), HistoryRefPath::REF_DIVIDER, path))
    }
}

impl Display for HistoryRefPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let path = Self::make_path_string(&self.ref_, &self.path)?;
        write!(f, "{}", path)
    }
}

trait UriSchemeChar {
    fn is_valid_uri_scheme_char(&self) -> bool;
}
// See https://www.rfc-editor.org/rfc/rfc3986#section-3.1
// scheme      = ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )
impl UriSchemeChar for char {
    fn is_valid_uri_scheme_char(&self) -> bool {
        self.is_ascii_alphanumeric() || matches!( *self, '-' | '.' | '+')
    }
}

fn is_valid_uri_scheme(scheme: &str) -> bool {
    let mut chars = scheme.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() => chars.all(|c| c.is_valid_uri_scheme_char()),
        _ => false,
    }
}

impl FromStr for HistoryRefPath {
    type Err = &'static str;
    fn from_str(path: &str) -> Result<Self, Self::Err> {
        let path = path.strip_prefix(HistoryRef::PATCHWORK_SCHEME_PREFIX).ok_or_else(|| "Invalid path")?;
        let (history_ref_part, path) = path.split_once(HistoryRefPath::REF_DIVIDER).ok_or_else(|| "Invalid path")?;
        // `simplify_path()` ends up mangling the uri identifier (e.g. `res://foo.gd` -> `res:/foo.gd`) so we need to check for that
        // TODO: remove this when this PR is merged and we rebase on that: (https://github.com/godotengine/godot/pull/115660)
        let path = if let Some(pos) = path.find(":/") {
            let uri_scheme = &path[..pos];
            // check if the previous characters before this were valid alphanumeric characters
            if is_valid_uri_scheme(uri_scheme) && path.len() >= pos+2 && &path[pos+2..pos+3] != "/" {
                // otherwise fix the path
                format!("{}://{}", uri_scheme.to_string(), path[pos+2..].to_string())
            } else{
                path.to_string()
            }
            
        } else {
            path.to_string()
        };
        let ref_ = HistoryRef::from_str(history_ref_part)?;
        Ok(HistoryRefPath { ref_, path })
    }
}
