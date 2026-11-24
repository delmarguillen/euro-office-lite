// Hyperlinks in a document reach the WebView as window.open(url) from the
// editor (web-apps DocumentHolderExt onHyperlinkClick). Without a handler the
// WebView drops the request on every platform, so the link does nothing.
// Here the request never becomes a window: http, https and mailto go to the
// system's default program and everything else is dropped.

const EXTERNAL_SCHEMES: [&str; 3] = ["http", "https", "mailto"];

// Exact, lowercase scheme as the WebView hands the URL over, so a value with a
// leading space or an unexpected spelling is refused rather than repaired.
pub fn is_external_link(url: &str) -> bool {
    let Some((scheme, _)) = url.split_once(':') else {
        return false;
    };
    if !EXTERNAL_SCHEMES.contains(&scheme) {
        return false;
    }
    url::Url::parse(url).is_ok()
}

#[allow(deprecated)]
fn open_in_system(url: &str) {
    // From Rust the shell plugin opens without a scope and the WebView gets no
    // shell permission (see capability_tests in main.rs).
    if let Err(e) = tauri_plugin_shell::open::open(None, url, None) {
        crate::log_startup(&format!("ERROR: open link: {e}"));
    }
}

pub fn handle_new_window<R: tauri::Runtime>(
    url: tauri::Url,
    _features: tauri::webview::NewWindowFeatures,
) -> tauri::webview::NewWindowResponse<R> {
    if is_external_link(url.as_str()) {
        open_in_system(url.as_str());
    }
    tauri::webview::NewWindowResponse::Deny
}

#[cfg(test)]
mod tests {
    use super::is_external_link;

    #[test]
    fn opens_web_and_mail_links() {
        assert!(is_external_link("https://example.com/"));
        assert!(is_external_link("http://example.com/a?b=c"));
        assert!(is_external_link("mailto:someone@example.com"));
        assert!(is_external_link("mailto:someone@example.com?subject=Hi%20there"));
    }

    #[test]
    fn refuses_other_schemes() {
        assert!(!is_external_link("file:///etc/passwd"));
        assert!(!is_external_link("javascript:alert(1)"));
        assert!(!is_external_link("ftp://example.com/"));
        assert!(!is_external_link("smb://server/share"));
        assert!(!is_external_link("tauri://localhost/index.html"));
    }

    #[test]
    fn refuses_empty_and_malformed() {
        assert!(!is_external_link(""));
        assert!(!is_external_link("mailto"));
        assert!(!is_external_link("https://"));
    }

    #[test]
    fn refuses_uppercase_and_spaces() {
        assert!(!is_external_link("HTTPS://example.com/"));
        assert!(!is_external_link("MailTo:someone@example.com"));
        assert!(!is_external_link(" https://example.com/"));
        assert!(!is_external_link("mailto :someone@example.com"));
    }
}
