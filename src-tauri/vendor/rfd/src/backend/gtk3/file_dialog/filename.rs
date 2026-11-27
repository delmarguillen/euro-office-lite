use std::ffi::CStr;
use std::os::raw::c_char;
use std::path::PathBuf;

// What gtk_file_chooser_get_filename returned, as a path. It returns NULL when
// no file is selected or the selected one has no local path: the portal file
// chooser accepts with no file when the portal answers success with no "uris".
pub unsafe fn path_from_filename(filename: *const c_char) -> Option<PathBuf> {
    if filename.is_null() {
        return None;
    }
    let name = CStr::from_ptr(filename).to_str().ok()?;
    Some(PathBuf::from(name))
}

#[cfg(test)]
mod tests {
    use super::path_from_filename;
    use std::ffi::CString;
    use std::path::PathBuf;

    #[test]
    fn a_null_filename_is_no_file() {
        let path = unsafe { path_from_filename(std::ptr::null()) };
        assert_eq!(path, None);
    }

    #[test]
    fn a_filename_is_its_path() {
        let name = CString::new("/docs/smoke.pdf").unwrap();
        let path = unsafe { path_from_filename(name.as_ptr()) };
        assert_eq!(path, Some(PathBuf::from("/docs/smoke.pdf")));
    }

    // As rfd 0.16.0 does: a name that is not UTF-8 is no file.
    #[test]
    fn a_filename_that_is_not_utf8_is_no_file() {
        let name = CString::new(vec![b'/', 0xff, b'a']).unwrap();
        let path = unsafe { path_from_filename(name.as_ptr()) };
        assert_eq!(path, None);
    }
}
