// Files the webview may have the backend read or write. A path gets here only
// from a native dialog the backend opened, or from the backend itself (the
// launch argument, the recent list, the open document). The webview gets an
// opaque token and the file name; the commands that touch a file take the
// token and look the path up here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

// Long enough for a reload between choosing a file and opening it.
pub const GRANT_TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrantKind {
    // A document to open in the editor.
    Open,
    // The file a Save As writes.
    Save,
    // A document converted to be inserted, compared or merged.
    Insert,
    // An image copied into the document's media.
    Image,
}

struct Grant {
    path: PathBuf,
    kind: GrantKind,
    expires: Instant,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct GrantedFile {
    pub token: String,
    pub name: String,
}

#[derive(Default)]
pub struct PathGrants {
    grants: Mutex<HashMap<String, Grant>>,
}

impl PathGrants {
    pub fn grant(&self, path: PathBuf, kind: GrantKind) -> GrantedFile {
        self.grant_at(path, kind, Instant::now())
    }

    pub fn resolve(&self, token: &str, kind: GrantKind) -> Result<PathBuf, String> {
        self.resolve_at(token, kind, Instant::now())
    }

    pub(crate) fn grant_at(&self, path: PathBuf, kind: GrantKind, now: Instant) -> GrantedFile {
        let token = new_token();
        let name = file_name(&path);
        let grant = Grant {
            path,
            kind,
            expires: now + GRANT_TTL,
        };
        let mut grants = self.grants.lock().unwrap();
        grants.retain(|_, grant| grant.expires > now);
        grants.insert(token.clone(), grant);
        GrantedFile { token, name }
    }

    fn resolve_at(&self, token: &str, kind: GrantKind, now: Instant) -> Result<PathBuf, String> {
        let mut grants = self.grants.lock().unwrap();
        grants.retain(|_, grant| grant.expires > now);
        let Some(grant) = grants.get(token) else {
            return Err("Unknown or expired file token".to_string());
        };
        if grant.kind != kind {
            return Err("The file token is for another use".to_string());
        }
        Ok(grant.path.clone())
    }
}

pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn new_token() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("no system random source");
    bytes.iter().map(|byte| format!("{:02x}", byte)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn granted(grants: &PathGrants, kind: GrantKind) -> GrantedFile {
        grants.grant(PathBuf::from("/home/u/Documents/report.docx"), kind)
    }

    #[test]
    fn a_granted_token_resolves_to_its_path() {
        let grants = PathGrants::default();
        let file = granted(&grants, GrantKind::Open);
        assert_eq!(file.name, "report.docx");
        assert_eq!(
            grants.resolve(&file.token, GrantKind::Open),
            Ok(PathBuf::from("/home/u/Documents/report.docx"))
        );
    }

    #[test]
    fn an_unknown_token_is_refused() {
        let grants = PathGrants::default();
        granted(&grants, GrantKind::Open);
        assert!(grants
            .resolve("0123456789abcdef0123456789abcdef", GrantKind::Open)
            .is_err());
        assert!(grants.resolve("", GrantKind::Open).is_err());
    }

    #[test]
    fn a_path_is_not_a_token() {
        let grants = PathGrants::default();
        granted(&grants, GrantKind::Insert);
        let path = "/home/u/Documents/report.docx";
        for kind in [
            GrantKind::Open,
            GrantKind::Save,
            GrantKind::Insert,
            GrantKind::Image,
        ] {
            assert!(grants.resolve(path, kind).is_err());
        }
    }

    #[test]
    fn a_token_of_another_kind_is_refused() {
        let grants = PathGrants::default();
        let image = grants.grant(PathBuf::from("/home/u/a.png"), GrantKind::Image);
        assert!(grants.resolve(&image.token, GrantKind::Insert).is_err());
        assert!(grants.resolve(&image.token, GrantKind::Open).is_err());
        let document = granted(&grants, GrantKind::Insert);
        assert!(grants.resolve(&document.token, GrantKind::Image).is_err());
        assert!(grants.resolve(&document.token, GrantKind::Save).is_err());
    }

    #[test]
    fn an_expired_token_is_refused() {
        let grants = PathGrants::default();
        let start = Instant::now();
        let file = grants.grant_at(PathBuf::from("/home/u/a.docx"), GrantKind::Open, start);
        assert!(grants
            .resolve_at(&file.token, GrantKind::Open, start + GRANT_TTL / 2)
            .is_ok());
        assert!(grants
            .resolve_at(&file.token, GrantKind::Open, start + GRANT_TTL)
            .is_err());
    }

    #[test]
    fn tokens_are_unique_and_say_nothing_of_the_path() {
        let grants = PathGrants::default();
        let first = granted(&grants, GrantKind::Open);
        let second = granted(&grants, GrantKind::Open);
        assert_ne!(first.token, second.token);
        assert_eq!(first.token.len(), 32);
        assert!(!first.token.contains("report"));
    }
}
