//! Minimal bindings to the stable CUPS 2 ABI. No shell, command-line tools,
//! locale-dependent output, or Windows print settings are involved.
use std::ffi::{CStr, CString};
use std::io::Read;
use std::os::raw::{c_char, c_int};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr;

#[repr(C)]
struct OptionValue {
    name: *mut c_char,
    value: *mut c_char,
}

#[repr(C)]
struct Destination {
    name: *mut c_char,
    instance: *mut c_char,
    is_default: c_int,
    num_options: c_int,
    options: *mut OptionValue,
}

#[link(name = "cups")]
extern "C" {
    fn cupsGetDests(dests: *mut *mut Destination) -> c_int;
    fn cupsFreeDests(count: c_int, dests: *mut Destination);
    fn cupsAddOption(
        name: *const c_char,
        value: *const c_char,
        count: c_int,
        options: *mut *mut OptionValue,
    ) -> c_int;
    fn cupsFreeOptions(count: c_int, options: *mut OptionValue);
    fn cupsPrintFile(
        name: *const c_char,
        filename: *const c_char,
        title: *const c_char,
        num_options: c_int,
        options: *mut OptionValue,
    ) -> c_int;
    fn cupsLastError() -> c_int;
    fn cupsLastErrorString() -> *const c_char;
}

// Function pointers let tests exercise allocation, submission, and errors
// without contacting a scheduler or sending anything to a physical printer.
struct Api {
    get_dests: unsafe extern "C" fn(*mut *mut Destination) -> c_int,
    free_dests: unsafe extern "C" fn(c_int, *mut Destination),
    print_file: unsafe extern "C" fn(
        *const c_char,
        *const c_char,
        *const c_char,
        c_int,
        *mut OptionValue,
    ) -> c_int,
    last_error: unsafe extern "C" fn() -> c_int,
    last_error_string: unsafe extern "C" fn() -> *const c_char,
}

const CUPS: Api = Api {
    get_dests: cupsGetDests,
    free_dests: cupsFreeDests,
    print_file: cupsPrintFile,
    last_error: cupsLastError,
    last_error_string: cupsLastErrorString,
};

#[derive(Debug, PartialEq)]
pub struct Printer {
    pub name: String,
    pub is_default: bool,
    pub color_supported: bool,
    pub duplex_supported: bool,
}

struct Destinations<'a> {
    api: &'a Api,
    count: c_int,
    ptr: *mut Destination,
}

#[derive(Default)]
struct JobOptions {
    count: c_int,
    ptr: *mut OptionValue,
}

impl JobOptions {
    fn add(&mut self, name: *const c_char, value: *const c_char) {
        // SAFETY: callers supply CUPS-owned or static NUL-terminated strings.
        // cupsAddOption copies them and inserts/replaces the named option in
        // a CUPS-allocated, sorted array owned by this guard.
        self.count = unsafe { cupsAddOption(name, value, self.count, &mut self.ptr) };
    }
}

impl Drop for JobOptions {
    fn drop(&mut self) {
        // SAFETY: this guard solely owns the array allocated by cupsAddOption.
        unsafe { cupsFreeOptions(self.count, self.ptr) };
    }
}

impl Drop for Destinations<'_> {
    fn drop(&mut self) {
        // SAFETY: these are the unmodified count and pointer from get_dests;
        // this guard is their sole owner and is dropped before Api goes away.
        unsafe { (self.api.free_dests)(self.count, self.ptr) };
    }
}

impl Api {
    fn error(&self, operation: &str) -> String {
        // SAFETY: CUPS owns this thread-local NUL-terminated string. Copy it
        // immediately, before any other CUPS call can replace it.
        let detail = unsafe {
            let message = (self.last_error_string)();
            if message.is_null() {
                "unknown CUPS error".to_owned()
            } else {
                CStr::from_ptr(message).to_string_lossy().into_owned()
            }
        };
        format!("CUPS {operation} failed: {detail}")
    }

    fn destinations(&self) -> Result<Destinations<'_>, String> {
        let mut dests = Destinations {
            api: self,
            count: 0,
            ptr: ptr::null_mut(),
        };
        // SAFETY: ptr points to initialized writable storage. CUPS allocates
        // the array; Destinations frees it on every return path.
        dests.count = unsafe { (self.get_dests)(&mut dests.ptr) };
        // IPP errors start at 0x0400; success may legitimately return no queues.
        if dests.count <= 0 && unsafe { (self.last_error)() } >= 0x0400 {
            return Err(self.error("printer discovery"));
        }
        if dests.count < 0 || (dests.count > 0 && dests.ptr.is_null()) {
            return Err("CUPS returned an invalid destination list".into());
        }
        Ok(dests)
    }

    fn get_printers(&self) -> Result<Vec<Printer>, String> {
        let dests = self.destinations()?;
        dests
            .entries()
            .iter()
            .map(|dest| {
                let name = destination_name(dest)?;
                let printer_type = option(dest, "printer-type")
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0);
                Ok(Printer {
                    name,
                    is_default: dest.is_default != 0,
                    color_supported: printer_type & 0x0008 != 0,
                    duplex_supported: printer_type & 0x0010 != 0,
                })
            })
            .collect()
    }

    fn print_pdf(&self, requested: &str, pdf: &Path) -> Result<i32, String> {
        // Reject invalid paths and non-PDF inputs before calling CUPS.
        let filename =
            CString::new(pdf.as_os_str().as_bytes()).map_err(|_| "PDF path contains NUL")?;
        let mut header = [0; 5];
        std::fs::File::open(pdf)
            .and_then(|mut f| f.read_exact(&mut header))
            .map_err(|e| e.to_string())?;
        if &header != b"%PDF-" {
            return Err("Print input is not a PDF".into());
        }
        let dests = self.destinations()?;
        let dest = dests
            .entries()
            .iter()
            .find(|dest| destination_name(dest).is_ok_and(|name| name == requested))
            .ok_or("Not a listed printer")?;
        // Use the base queue plus the saved instance's options. cupsPrintFile
        // expects a queue name, not a queue/instance destination string.
        let mut options = JobOptions::default();
        if dest.num_options > 0 && !dest.options.is_null() {
            // SAFETY: CUPS supplies num_options entries owned by dests.
            for option in
                unsafe { std::slice::from_raw_parts(dest.options, dest.num_options as usize) }
            {
                options.add(option.name, option.value);
            }
        }
        // Preserve the document's dimensions instead of shrinking the whole
        // PDF page to the printer's printable area. Override any queue or
        // instance scaling default without modifying the saved destination.
        options.add(
            b"print-scaling\0".as_ptr().cast(),
            b"none\0".as_ptr().cast(),
        );
        // SAFETY: dests, filename, and options stay alive throughout this
        // synchronous call. CUPS spools the file before returning a job ID.
        let job = unsafe {
            (self.print_file)(
                dest.name,
                filename.as_ptr(),
                b"Euro-Office Lite\0".as_ptr().cast(),
                options.count,
                options.ptr,
            )
        };
        if job <= 0 {
            Err(self.error("PDF submission"))
        } else {
            Ok(job)
        }
    }
}

impl Destinations<'_> {
    fn entries(&self) -> &[Destination] {
        if self.count == 0 {
            &[]
        } else {
            // SAFETY: get_dests supplies count initialized entries; the guard
            // owns the allocation for the entire borrow.
            unsafe { std::slice::from_raw_parts(self.ptr, self.count as usize) }
        }
    }
}

fn destination_name(dest: &Destination) -> Result<String, String> {
    if dest.name.is_null() {
        return Err("CUPS returned a nameless printer".into());
    }
    // SAFETY: CUPS destination strings remain valid while their list is owned.
    let mut name = unsafe { CStr::from_ptr(dest.name) }
        .to_str()
        .map_err(|e| e.to_string())?
        .to_owned();
    if !dest.instance.is_null() {
        name.push('/');
        name.push_str(
            unsafe { CStr::from_ptr(dest.instance) }
                .to_str()
                .map_err(|e| e.to_string())?,
        );
    }
    Ok(name)
}

fn option(dest: &Destination, requested: &str) -> Option<String> {
    if dest.num_options <= 0 || dest.options.is_null() {
        return None;
    }
    // SAFETY: CUPS supplies num_options entries belonging to the destination.
    unsafe { std::slice::from_raw_parts(dest.options, dest.num_options as usize) }
        .iter()
        .find_map(|opt| {
            if opt.name.is_null() || opt.value.is_null() {
                return None;
            }
            // SAFETY: non-null option strings are NUL-terminated by CUPS.
            unsafe {
                (CStr::from_ptr(opt.name).to_bytes() == requested.as_bytes())
                    .then(|| CStr::from_ptr(opt.value).to_string_lossy().into_owned())
            }
        })
}

pub fn get_printers() -> Result<Vec<Printer>, String> {
    CUPS.get_printers()
}

pub fn print_pdf(printer: &str, pdf: &Path) -> Result<i32, String> {
    CUPS.print_pdf(printer, pdf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    thread_local! {
        static STATE: RefCell<TestState> = RefCell::new(TestState::default());
    }
    #[derive(Default)]
    struct TestState {
        empty: bool,
        error: c_int,
        job: c_int,
        freed: usize,
        submitted: Vec<(String, Vec<u8>, Vec<(String, String)>)>,
    }

    unsafe extern "C" fn get(dests: *mut *mut Destination) -> c_int {
        if STATE.with(|s| s.borrow().empty) {
            return 0;
        }
        let opts = Box::new([
            OptionValue {
                name: b"printer-type\0".as_ptr().cast_mut().cast(),
                value: b"24\0".as_ptr().cast_mut().cast(),
            },
            OptionValue {
                name: b"print-scaling\0".as_ptr().cast_mut().cast(),
                value: b"fit\0".as_ptr().cast_mut().cast(),
            },
            OptionValue {
                name: b"sides\0".as_ptr().cast_mut().cast(),
                value: b"two-sided-long-edge\0".as_ptr().cast_mut().cast(),
            },
        ]);
        let list = Box::new([
            Destination {
                name: b"Office\0".as_ptr().cast_mut().cast(),
                instance: ptr::null_mut(),
                is_default: 0,
                num_options: 0,
                options: ptr::null_mut(),
            },
            Destination {
                name: b"Office\0".as_ptr().cast_mut().cast(),
                instance: b"Duplex\0".as_ptr().cast_mut().cast(),
                is_default: 1,
                num_options: 3,
                options: Box::into_raw(opts).cast(),
            },
        ]);
        *dests = Box::into_raw(list).cast();
        2
    }
    unsafe extern "C" fn free(count: c_int, dests: *mut Destination) {
        if count == 2 {
            let list = Box::from_raw(dests.cast::<[Destination; 2]>());
            drop(Box::from_raw(list[1].options.cast::<[OptionValue; 3]>()));
        }
        STATE.with(|s| s.borrow_mut().freed += 1);
    }
    unsafe extern "C" fn print(
        name: *const c_char,
        file: *const c_char,
        _: *const c_char,
        count: c_int,
        options: *mut OptionValue,
    ) -> c_int {
        STATE.with(|s| {
            let mut state = s.borrow_mut();
            state.submitted.push((
                CStr::from_ptr(name).to_str().unwrap().into(),
                CStr::from_ptr(file).to_bytes().to_vec(),
                std::slice::from_raw_parts(options, count as usize)
                    .iter()
                    .map(|option| {
                        (
                            CStr::from_ptr(option.name).to_str().unwrap().into(),
                            CStr::from_ptr(option.value).to_str().unwrap().into(),
                        )
                    })
                    .collect(),
            ));
            state.job
        })
    }
    unsafe extern "C" fn error() -> c_int {
        STATE.with(|s| s.borrow().error)
    }
    unsafe extern "C" fn error_string() -> *const c_char {
        b"Scheduler unavailable\0".as_ptr().cast()
    }
    fn api() -> Api {
        STATE.with(|s| {
            *s.borrow_mut() = TestState {
                job: 42,
                ..Default::default()
            }
        });
        Api {
            get_dests: get,
            free_dests: free,
            print_file: print,
            last_error: error,
            last_error_string: error_string,
        }
    }
    struct Pdf(std::path::PathBuf);
    impl Pdf {
        fn new() -> Self {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "printer-test-{}-{} '$(touch ignored).pdf",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::write(&path, b"%PDF-1.7\nfixture").unwrap();
            Self(path)
        }
    }
    impl Drop for Pdf {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn lists_instances_default_and_capabilities_and_frees_allocation() {
        let printers = api().get_printers().unwrap();
        assert_eq!(
            printers,
            vec![
                Printer {
                    name: "Office".into(),
                    is_default: false,
                    color_supported: false,
                    duplex_supported: false
                },
                Printer {
                    name: "Office/Duplex".into(),
                    is_default: true,
                    color_supported: true,
                    duplex_supported: true
                },
            ]
        );
        STATE.with(|s| assert_eq!(s.borrow().freed, 1));
    }
    #[test]
    fn distinguishes_no_printers_from_scheduler_failure() {
        let api = api();
        STATE.with(|s| s.borrow_mut().empty = true);
        assert!(api.get_printers().unwrap().is_empty());
        STATE.with(|s| s.borrow_mut().error = 0x0502);
        assert!(api
            .get_printers()
            .unwrap_err()
            .contains("Scheduler unavailable"));
        STATE.with(|s| assert_eq!(s.borrow().freed, 2));
    }
    #[test]
    fn submits_instance_options_and_literal_path_without_deleting_pdf() {
        let api = api();
        let pdf = Pdf::new();
        assert_eq!(api.print_pdf("Office/Duplex", &pdf.0).unwrap(), 42);
        assert!(pdf.0.exists());
        STATE.with(|s| {
            assert_eq!(
                s.borrow().submitted,
                vec![(
                    "Office".into(),
                    pdf.0.as_os_str().as_bytes().to_vec(),
                    vec![
                        ("print-scaling".into(), "none".into()),
                        ("printer-type".into(), "24".into()),
                        ("sides".into(), "two-sided-long-edge".into()),
                    ]
                )]
            );
            assert_eq!(s.borrow().freed, 1);
        });
    }
    #[test]
    fn submits_no_scaling_when_destination_has_no_options() {
        let api = api();
        let pdf = Pdf::new();
        assert_eq!(api.print_pdf("Office", &pdf.0).unwrap(), 42);
        STATE.with(|s| {
            assert_eq!(
                s.borrow().submitted[0].2,
                vec![("print-scaling".into(), "none".into())]
            );
        });
    }
    #[test]
    fn refuses_unknown_and_injected_names_without_submitting() {
        let api = api();
        let pdf = Pdf::new();
        for name in [
            "Missing",
            "Office; touch /tmp/ignored",
            "Office\0",
            "office",
            "Office/Other",
        ] {
            assert_eq!(
                api.print_pdf(name, &pdf.0).unwrap_err(),
                "Not a listed printer"
            );
        }
        STATE.with(|s| assert!(s.borrow().submitted.is_empty()));
    }
    #[test]
    fn failed_submission_keeps_pdf_and_reports_cups_error() {
        let api = api();
        STATE.with(|s| s.borrow_mut().job = 0);
        let pdf = Pdf::new();
        assert!(api
            .print_pdf("Office", &pdf.0)
            .unwrap_err()
            .contains("Scheduler unavailable"));
        assert!(pdf.0.exists());
        STATE.with(|s| assert_eq!(s.borrow().freed, 1));
    }
    #[test]
    fn refuses_missing_and_non_pdf_files_before_submission() {
        let api = api();
        let pdf = Pdf::new();
        std::fs::write(&pdf.0, b"not a pdf").unwrap();
        assert_eq!(
            api.print_pdf("Office", &pdf.0).unwrap_err(),
            "Print input is not a PDF"
        );
        std::fs::remove_file(&pdf.0).unwrap();
        assert!(api.print_pdf("Office", &pdf.0).is_err());
        STATE.with(|s| assert!(s.borrow().submitted.is_empty()));
    }
}
