// What each route of the ascdesktop protocol accepts. Every rule is a plain
// function so it can be tested without a running app.

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

// A path under the resource directory: only plain names, so no root, drive
// prefix (\\?\ included) or ".." can move the join outside it.
pub fn resource_relative(path: &str) -> Option<PathBuf> {
    let path = Path::new(path);
    let mut relative = PathBuf::new();
    for component in path.components() {
        let Component::Normal(name) = component else {
            return None;
        };
        relative.push(name);
    }
    if relative.as_os_str().is_empty() {
        return None;
    }
    Some(relative)
}

// The image types the insert dialog offers and copy-to-media copies.
pub const IMAGE_EXTENSIONS: [&str; 11] = [
    "png", "jpg", "jpeg", "gif", "bmp", "webp", "svg", "tif", "tiff", "emf", "wmf",
];

pub fn has_image_extension(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
        return false;
    };
    IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
}

// An absolute path copy-to-media may read: one the user picked in the image
// dialog, and an image.
pub fn may_copy_absolute(path: &Path, picked: &[PathBuf]) -> bool {
    if !picked.iter().any(|choice| choice == path) {
        return false;
    }
    has_image_extension(path)
}

pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15);
pub const DOWNLOAD_MAX_BYTES: u64 = 50 * 1024 * 1024;
const DOWNLOAD_MAX_REDIRECTS: usize = 5;

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    !(ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_unspecified()
        || ip.is_broadcast())
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_ipv4(v4);
    }
    let first = ip.segments()[0];
    let link_local = first & 0xffc0 == 0xfe80;
    let site_local = first & 0xffc0 == 0xfec0;
    let unique_local = first & 0xfe00 == 0xfc00;
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || link_local
        || site_local
        || unique_local)
}

// An address a download may connect to: not this machine, not the local
// network, not multicast.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_ipv4(v4),
        IpAddr::V6(v6) => is_public_ipv6(v6),
    }
}

// A host is allowed when it resolves and every address it resolves to is
// public, so a name that mixes in a private address is refused.
pub fn host_allowed(addrs: &[IpAddr]) -> bool {
    !addrs.is_empty() && addrs.iter().all(|ip| is_public_ip(*ip))
}

pub fn is_download_scheme(url: &url::Url) -> bool {
    matches!(url.scheme(), "http" | "https")
}

pub fn is_image_content_type(content_type: &str) -> bool {
    let essence = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    let Some(subtype) = essence.strip_prefix("image/") else {
        return false;
    };
    !subtype.is_empty()
}

fn check_download_url(url: &url::Url) -> Result<(), String> {
    if !is_download_scheme(url) {
        return Err(format!("scheme not allowed: {}", url.scheme()));
    }
    let addrs: Vec<IpAddr> = url
        .socket_addrs(|| None)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|addr| addr.ip())
        .collect();
    if !host_allowed(&addrs) {
        return Err(format!("host not allowed: {}", url.host_str().unwrap_or("")));
    }
    Ok(())
}

// Downloads an image for download-to-media. Redirects are followed here, one
// hop at a time, so every host is checked before it is contacted.
pub fn download_image(start: &str) -> Result<Vec<u8>, String> {
    let deadline = Instant::now() + DOWNLOAD_TIMEOUT;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .into();
    let mut url = url::Url::parse(start).map_err(|e| e.to_string())?;
    for _ in 0..=DOWNLOAD_MAX_REDIRECTS {
        check_download_url(&url)?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("timed out".to_string());
        }
        let response = agent
            .get(url.as_str())
            .config()
            .timeout_global(Some(remaining))
            .build()
            .call()
            .map_err(|e| e.to_string())?;
        let status = response.status();
        if status.is_redirection() {
            let Some(location) = response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok())
            else {
                return Err(format!("redirect {} without a location", status));
            };
            url = url.join(location).map_err(|e| e.to_string())?;
            continue;
        }
        if !status.is_success() {
            return Err(format!("status {}", status));
        }
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        if !is_image_content_type(content_type) {
            return Err(format!("not an image: {}", content_type));
        }
        return response
            .into_body()
            .with_config()
            .limit(DOWNLOAD_MAX_BYTES)
            .read_to_vec()
            .map_err(|e| e.to_string());
    }
    Err("too many redirects".to_string())
}

// The strings of window["__fonts_files"] = [ "path", ... ] in AllFonts.js.
// Escapes are taken as the character they escape, as x2t writes them.
pub fn font_files(all_fonts_js: &str) -> Vec<String> {
    let marker = "window[\"__fonts_files\"]";
    let Some(start) = all_fonts_js.find(marker) else {
        return Vec::new();
    };
    let rest = &all_fonts_js[start + marker.len()..];
    let Some(open) = rest.find('[') else {
        return Vec::new();
    };
    let mut files = Vec::new();
    let mut current: Option<String> = None;
    let mut chars = rest[open + 1..].chars();
    while let Some(c) = chars.next() {
        match (current.as_mut(), c) {
            (None, ']') => break,
            (None, '"') => current = Some(String::new()),
            (None, _) => {}
            (Some(text), '\\') => text.extend(chars.next()),
            (Some(_), '"') => files.extend(current.take()),
            (Some(text), _) => text.push(c),
        }
    }
    files
}

// One spelling for a font path: forward slashes, without the extended prefix
// x2t may write as \\?\ or /?/.
pub fn font_path_key(path: &str) -> String {
    let path = path.replace('\\', "/");
    let plain = path
        .strip_prefix("//?/")
        .or_else(|| path.strip_prefix("/?/"))
        .unwrap_or(&path);
    plain.to_string()
}

pub fn is_listed_font(requested: &str, listed: &HashSet<String>) -> bool {
    listed.contains(&font_path_key(requested))
}

// The font list of the generated AllFonts.js, read again only when the file
// changes.
static FONT_LIST: Mutex<Option<(SystemTime, HashSet<String>)>> = Mutex::new(None);

pub fn is_generated_font(all_fonts_js: &Path, requested: &str) -> bool {
    let Ok(modified) = std::fs::metadata(all_fonts_js).and_then(|meta| meta.modified()) else {
        return false;
    };
    let mut cache = FONT_LIST.lock().unwrap();
    let fresh = matches!(cache.as_ref(), Some((seen, _)) if *seen == modified);
    if !fresh {
        let Ok(text) = std::fs::read_to_string(all_fonts_js) else {
            return false;
        };
        let listed = font_files(&text).iter().map(|file| font_path_key(file)).collect();
        *cache = Some((modified, listed));
    }
    let Some((_, listed)) = cache.as_ref() else {
        return false;
    };
    is_listed_font(requested, listed)
}

// Largest Editor.bin the JS may send, decoded. A large document's
// Editor.bin can pass 100 MB; this leaves room above that.
pub const EDITOR_BIN_MAX_BYTES: usize = 512 * 1024 * 1024;

// Largest log line kept from the JS; longer lines are cut.
pub const JS_LOG_MAX_BYTES: usize = 16 * 1024;

// Refuses base64 text that would decode to more than `max_bytes`, before
// decoding it.
pub fn check_base64_size(what: &str, data: &str, max_bytes: usize) -> Result<(), String> {
    let decoded = data.len() / 4 * 3;
    if decoded <= max_bytes {
        return Ok(());
    }
    Err(format!(
        "{what} is too large: about {decoded} bytes, limit {max_bytes} bytes"
    ))
}

// Cuts a log line to at most `max_bytes`, on a character boundary.
pub fn truncate_log(msg: &str, max_bytes: usize) -> &str {
    if msg.len() <= max_bytes {
        return msg;
    }
    let mut end = max_bytes;
    while !msg.is_char_boundary(end) {
        end -= 1;
    }
    &msg[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_resource_path_is_accepted() {
        assert_eq!(
            resource_relative("sdkjs/word/sdk-all.js"),
            Some(PathBuf::from("sdkjs/word/sdk-all.js"))
        );
        assert_eq!(resource_relative("index.html"), Some(PathBuf::from("index.html")));
    }

    #[test]
    fn a_current_dir_component_is_refused() {
        assert_eq!(resource_relative("./fonts/a.ttf"), None);
    }

    #[test]
    fn a_parent_component_is_refused() {
        assert_eq!(resource_relative("../secret"), None);
        assert_eq!(resource_relative("sdkjs/../../etc/passwd"), None);
    }

    #[test]
    fn an_absolute_path_is_refused() {
        assert_eq!(resource_relative("/etc/passwd"), None);
    }

    #[test]
    fn an_empty_path_is_refused() {
        assert_eq!(resource_relative(""), None);
    }

    #[cfg(windows)]
    #[test]
    fn a_drive_or_verbatim_path_is_refused() {
        assert_eq!(resource_relative("C:\\Windows\\win.ini"), None);
        assert_eq!(resource_relative("C:win.ini"), None);
        assert_eq!(resource_relative("\\\\?\\C:\\Windows\\win.ini"), None);
        assert_eq!(resource_relative("\\\\server\\share\\x"), None);
    }

    #[test]
    fn a_picked_image_may_be_copied() {
        let picked = vec![PathBuf::from("/home/u/Pictures/a.PNG"), PathBuf::from("/home/u/b.emf")];
        assert!(may_copy_absolute(Path::new("/home/u/Pictures/a.PNG"), &picked));
        assert!(may_copy_absolute(Path::new("/home/u/b.emf"), &picked));
    }

    #[test]
    fn a_path_that_was_not_picked_is_refused() {
        let picked = vec![PathBuf::from("/home/u/Pictures/a.png")];
        assert!(!may_copy_absolute(Path::new("/home/u/Pictures/b.png"), &picked));
        assert!(!may_copy_absolute(Path::new("/home/u/Pictures/a.png"), &[]));
    }

    #[test]
    fn a_picked_file_that_is_not_an_image_is_refused() {
        let picked = vec![PathBuf::from("/home/u/.ssh/id_rsa"), PathBuf::from("/home/u/notes.txt")];
        assert!(!may_copy_absolute(Path::new("/home/u/.ssh/id_rsa"), &picked));
        assert!(!may_copy_absolute(Path::new("/home/u/notes.txt"), &picked));
    }

    #[test]
    fn image_extensions_ignore_case() {
        assert!(has_image_extension(Path::new("x.JPeG")));
        assert!(has_image_extension(Path::new("x.tiff")));
        assert!(!has_image_extension(Path::new("x.ico")));
        assert!(!has_image_extension(Path::new("png")));
    }

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn a_public_address_may_be_downloaded_from() {
        assert!(is_public_ip(ip("93.184.216.34")));
        assert!(is_public_ip(ip("2606:2800:220:1:248:1893:25c8:1946")));
        assert!(is_public_ip(ip("::ffff:93.184.216.34")));
    }

    #[test]
    fn local_and_multicast_ipv4_addresses_are_refused() {
        for text in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "224.0.0.1",
            "0.0.0.0",
            "255.255.255.255",
        ] {
            assert!(!is_public_ip(ip(text)), "{}", text);
        }
    }

    #[test]
    fn local_and_multicast_ipv6_addresses_are_refused() {
        for text in [
            "::1",
            "::",
            "fe80::1",
            "fc00::1",
            "fd12:3456::1",
            "fec0::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:192.168.0.1",
        ] {
            assert!(!is_public_ip(ip(text)), "{}", text);
        }
    }

    #[test]
    fn a_host_with_any_private_address_is_refused() {
        assert!(host_allowed(&[ip("93.184.216.34")]));
        assert!(!host_allowed(&[ip("93.184.216.34"), ip("10.0.0.1")]));
        assert!(!host_allowed(&[]));
    }

    #[test]
    fn only_http_and_https_are_downloaded() {
        let parse = |text: &str| url::Url::parse(text).unwrap();
        assert!(is_download_scheme(&parse("http://example.com/a.png")));
        assert!(is_download_scheme(&parse("https://example.com/a.png")));
        assert!(!is_download_scheme(&parse("file:///etc/passwd")));
        assert!(!is_download_scheme(&parse("ftp://example.com/a.png")));
    }

    #[test]
    fn a_literal_local_host_is_refused_before_connecting() {
        let parse = |text: &str| url::Url::parse(text).unwrap();
        assert!(check_download_url(&parse("http://127.0.0.1:8080/a.png")).is_err());
        assert!(check_download_url(&parse("http://[::1]/a.png")).is_err());
        assert!(check_download_url(&parse("http://169.254.169.254/latest")).is_err());
        assert!(check_download_url(&parse("http://93.184.216.34/a.png")).is_ok());
    }

    #[test]
    fn only_image_content_types_are_kept() {
        assert!(is_image_content_type("image/png"));
        assert!(is_image_content_type("Image/JPEG; charset=binary"));
        assert!(!is_image_content_type("text/html"));
        assert!(!is_image_content_type("image/"));
        assert!(!is_image_content_type(""));
    }

    const ALL_FONTS: &str = "\u{feff}window[\"__all_fonts_js_version__\"] = 2;\n\nwindow[\"__fonts_files\"] = [\n\"/usr/share/fonts/DejaVuSans.ttf\",\n\"C:\\\\Windows\\\\Fonts\\\\arial.ttf\",\n\"/?/C:/Users/u/AppData/Local/Microsoft/Windows/Fonts/My \\\"Font\\\".otf\"\n];\nwindow[\"__fonts_infos\"] = [\n[\"DejaVu Sans\",0,0,-1,-1,-1,-1,-1,-1]\n];\n";

    fn listed() -> HashSet<String> {
        font_files(ALL_FONTS).iter().map(|file| font_path_key(file)).collect()
    }

    #[test]
    fn the_font_files_array_is_read_with_its_escapes() {
        assert_eq!(
            font_files(ALL_FONTS),
            vec![
                "/usr/share/fonts/DejaVuSans.ttf".to_string(),
                "C:\\Windows\\Fonts\\arial.ttf".to_string(),
                "/?/C:/Users/u/AppData/Local/Microsoft/Windows/Fonts/My \"Font\".otf".to_string(),
            ]
        );
        assert!(font_files("window[\"__fonts_infos\"] = [];").is_empty());
    }

    #[test]
    fn a_listed_font_is_served_in_any_spelling() {
        let listed = listed();
        assert!(is_listed_font("/usr/share/fonts/DejaVuSans.ttf", &listed));
        assert!(is_listed_font("C:/Windows/Fonts/arial.ttf", &listed));
        assert!(is_listed_font("C:\\Windows\\Fonts\\arial.ttf", &listed));
        assert!(is_listed_font("\\\\?\\C:\\Windows\\Fonts\\arial.ttf", &listed));
        assert!(is_listed_font(
            "C:/Users/u/AppData/Local/Microsoft/Windows/Fonts/My \"Font\".otf",
            &listed
        ));
        assert!(is_listed_font(
            "/?/C:/Users/u/AppData/Local/Microsoft/Windows/Fonts/My \"Font\".otf",
            &listed
        ));
    }

    #[test]
    fn a_file_that_is_not_listed_is_refused() {
        let listed = listed();
        assert!(!is_listed_font("/etc/passwd", &listed));
        assert!(!is_listed_font("/usr/share/fonts/../../../etc/passwd", &listed));
        assert!(!is_listed_font("/usr/share/fonts/Other.ttf", &listed));
        assert!(!is_listed_font("C:/Windows/win.ini", &listed));
    }

    #[test]
    fn base64_within_the_limit_is_accepted() {
        assert!(check_base64_size("Editor.bin", "AAAA", 3).is_ok());
        assert!(check_base64_size("Editor.bin", "", 0).is_ok());
    }

    #[test]
    fn base64_over_the_limit_is_refused() {
        let error = check_base64_size("Editor.bin", "AAAAAAAA", 5).unwrap_err();
        assert!(error.contains("Editor.bin is too large"));
        assert!(error.contains("limit 5 bytes"));
    }

    #[test]
    fn a_short_log_line_is_kept() {
        assert_eq!(truncate_log("hello", 16), "hello");
    }

    #[test]
    fn a_long_log_line_is_cut() {
        assert_eq!(truncate_log("abcdef", 4), "abcd");
    }

    #[test]
    fn a_log_line_is_cut_on_a_character_boundary() {
        // "é" is two bytes; cutting at byte 2 would split it.
        assert_eq!(truncate_log("aé", 2), "a");
    }
}
