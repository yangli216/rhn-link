//! RHN controlled PDF printing adapter for the isolated plugin host.
//!
//! The exported ABI uses only UTF-8 strings, one 32-bit integer and one
//! caller-owned output buffer so it can be loaded through the generic
//! `webplus-native` DLL mapping. Every export returns a stable status code and
//! writes a JSON document into the declared output buffer; see
//! `docs/rhn-printing.md` for the public contract.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Output buffer length declared for `listPrinters` in `api.json`.
pub const LIST_OUTPUT_CAPACITY: usize = 64 * 1024;
/// Output buffer length declared for `printPdf` in `api.json`.
pub const PRINT_OUTPUT_CAPACITY: usize = 1024;
pub const MAX_DOCUMENT_BYTES: usize = 6 * 1024 * 1024;
pub const MAX_DOCUMENT_BASE64_CHARS: usize = MAX_DOCUMENT_BYTES.div_ceil(3) * 4;
pub const MAX_PRINTER_NAME_CHARS: usize = 256;
pub const MAX_JOB_NAME_CHARS: usize = 128;
pub const MAX_COPIES: i32 = 99;
/// Upper bound for `pages * copies` in one job, keeping a single call well
/// inside the declared method timeout.
pub const MAX_PRINTED_PAGES: u64 = 200;
const MAX_SHORT_INPUT_BYTES: usize = 4 * 1024;
const PDF_HEADER: &[u8] = b"%PDF-";

/// Stable `ReturnValue` codes shared with RHN business code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    InvalidRequest,
    DocumentDigestMismatch,
    DocumentInvalid,
    PrinterNotFound,
    PrinterUnavailable,
    JobAborted,
    OutputBufferTooSmall,
    Internal,
    UnsupportedPlatform,
}

impl Failure {
    pub const fn code(self) -> usize {
        match self {
            Self::InvalidRequest => 1001,
            Self::DocumentDigestMismatch => 1002,
            Self::DocumentInvalid => 1003,
            Self::PrinterNotFound => 1004,
            Self::PrinterUnavailable => 1005,
            Self::JobAborted => 1006,
            Self::OutputBufferTooSmall => 1007,
            Self::Internal => 1008,
            Self::UnsupportedPlatform => 1050,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid-request",
            Self::DocumentDigestMismatch => "document-digest-mismatch",
            Self::DocumentInvalid => "document-invalid",
            Self::PrinterNotFound => "printer-not-found",
            Self::PrinterUnavailable => "printer-unavailable",
            Self::JobAborted => "job-aborted",
            Self::OutputBufferTooSmall => "output-buffer-too-small",
            Self::Internal => "internal-error",
            Self::UnsupportedPlatform => "unsupported-platform",
        }
    }
}

const SUCCESS: usize = 0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Printer {
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrintRequest {
    pub printer_name: String,
    pub document: Vec<u8>,
    pub copies: u32,
    pub job_name: String,
}

/// Validates every field and verifies the digest before any printer is
/// touched. Rejections here guarantee that no print job was created.
pub fn validate_print_request(
    printer_name: &str,
    document_base64: &str,
    sha256: &str,
    copies: i32,
    job_name: &str,
) -> Result<PrintRequest, Failure> {
    if !is_bounded_label(printer_name, MAX_PRINTER_NAME_CHARS)
        || !is_bounded_label(job_name, MAX_JOB_NAME_CHARS)
        || !(1..=MAX_COPIES).contains(&copies)
        || !is_lowercase_sha256(sha256)
    {
        return Err(Failure::InvalidRequest);
    }
    if document_base64.is_empty() || document_base64.len() > MAX_DOCUMENT_BASE64_CHARS {
        return Err(Failure::DocumentInvalid);
    }
    let document = BASE64
        .decode(document_base64)
        .map_err(|_| Failure::DocumentInvalid)?;
    if document.is_empty() || document.len() > MAX_DOCUMENT_BYTES {
        return Err(Failure::DocumentInvalid);
    }
    if hex_lower(&Sha256::digest(&document)) != sha256 {
        return Err(Failure::DocumentDigestMismatch);
    }
    if !document.starts_with(PDF_HEADER) {
        return Err(Failure::DocumentInvalid);
    }
    Ok(PrintRequest {
        printer_name: printer_name.to_owned(),
        document,
        copies: copies as u32,
        job_name: job_name.to_owned(),
    })
}

/// Resolves the caller's printer name against the enumerated queues. Windows
/// printer names are case-insensitive; the enumerated spelling is returned.
pub fn resolve_printer<'a>(printers: &'a [Printer], requested: &str) -> Option<&'a Printer> {
    printers
        .iter()
        .find(|printer| printer.name == requested)
        .or_else(|| {
            let requested = requested.to_lowercase();
            printers
                .iter()
                .find(|printer| printer.name.to_lowercase() == requested)
        })
}

pub fn check_page_budget(page_count: u32, copies: u32) -> Result<(), Failure> {
    let total = u64::from(page_count) * u64::from(copies);
    if page_count == 0 || total > MAX_PRINTED_PAGES {
        return Err(Failure::DocumentInvalid);
    }
    Ok(())
}

/// Printer device geometry from `GetDeviceCaps`, in device pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceGeometry {
    pub dpi_x: i32,
    pub dpi_y: i32,
    pub physical_width: i32,
    pub physical_height: i32,
    pub offset_x: i32,
    pub offset_y: i32,
}

/// Where one PDF page lands on the device and the bitmap size to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageLayout {
    pub dest_x: i32,
    pub dest_y: i32,
    pub dest_width: i32,
    pub dest_height: i32,
    pub render_width: u32,
    pub render_height: u32,
}

pub const MAX_RENDER_DPI: f64 = 300.0;
pub const MAX_RENDER_PIXELS: f64 = 48_000_000.0;
const DIPS_PER_INCH: f64 = 96.0;

/// Places a page at its real size centred on the physical sheet, shrinking it
/// proportionally when it is larger than the sheet.
pub fn layout_page(
    page_width_dips: f64,
    page_height_dips: f64,
    device: DeviceGeometry,
) -> Result<PageLayout, Failure> {
    let finite_positive = |value: f64| value.is_finite() && value > 0.0;
    if !finite_positive(page_width_dips)
        || !finite_positive(page_height_dips)
        || device.dpi_x <= 0
        || device.dpi_y <= 0
        || device.physical_width <= 0
        || device.physical_height <= 0
        || device.offset_x < 0
        || device.offset_y < 0
    {
        return Err(Failure::PrinterUnavailable);
    }
    let natural_width = page_width_dips / DIPS_PER_INCH * f64::from(device.dpi_x);
    let natural_height = page_height_dips / DIPS_PER_INCH * f64::from(device.dpi_y);
    let fit = (f64::from(device.physical_width) / natural_width)
        .min(f64::from(device.physical_height) / natural_height)
        .min(1.0);
    let dest_width = (natural_width * fit).round().max(1.0);
    let dest_height = (natural_height * fit).round().max(1.0);
    let dest_x =
        ((f64::from(device.physical_width) - dest_width) / 2.0).floor() as i32 - device.offset_x;
    let dest_y =
        ((f64::from(device.physical_height) - dest_height) / 2.0).floor() as i32 - device.offset_y;

    let mut render_scale = (MAX_RENDER_DPI / f64::from(device.dpi_x))
        .min(MAX_RENDER_DPI / f64::from(device.dpi_y))
        .min(1.0);
    let pixels = dest_width * dest_height * render_scale * render_scale;
    if pixels > MAX_RENDER_PIXELS {
        render_scale *= (MAX_RENDER_PIXELS / pixels).sqrt();
    }
    let render_width = (dest_width * render_scale).floor().max(1.0);
    let render_height = (dest_height * render_scale).floor().max(1.0);
    Ok(PageLayout {
        dest_x,
        dest_y,
        dest_width: dest_width as i32,
        dest_height: dest_height as i32,
        render_width: render_width as u32,
        render_height: render_height as u32,
    })
}

pub fn printers_document(printers: &[Printer]) -> String {
    let printers = printers
        .iter()
        .map(|printer| json!({ "name": printer.name, "isDefault": printer.is_default }))
        .collect::<Vec<_>>();
    json!({ "printers": printers }).to_string()
}

pub fn submitted_document(system_job_id: Option<i32>) -> String {
    let mut document = json!({ "submitted": true });
    if let Some(job_id) = system_job_id.filter(|job_id| *job_id > 0) {
        document["systemJobId"] = Value::from(job_id);
    }
    document.to_string()
}

pub fn error_document(failure: Failure) -> String {
    json!({ "error": failure.name() }).to_string()
}

fn is_bounded_label(value: &str, max_chars: usize) -> bool {
    let count = value.chars().count();
    (1..=max_chars).contains(&count)
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn hex_lower(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[usize::from(byte >> 4)] as char);
        output.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    output
}

#[cfg(windows)]
mod platform;

#[cfg(not(windows))]
mod platform {
    use super::{Failure, PrintRequest, Printer};

    pub fn list_printers() -> Result<Vec<Printer>, Failure> {
        Err(Failure::UnsupportedPlatform)
    }

    pub fn print_pdf(_request: &PrintRequest) -> Result<Option<i32>, Failure> {
        Err(Failure::UnsupportedPlatform)
    }
}

/// Writes `{"printers":[{"name","isDefault"}]}` into `output`.
///
/// # Safety
///
/// `output` must point to the 65,536-byte writable buffer declared in
/// `api.json`.
#[export_name = "RhnListPrinters"]
pub unsafe extern "C" fn rhn_list_printers(output: *mut u8) -> usize {
    let result = guarded(|| platform::list_printers().map(|printers| printers_document(&printers)));
    unsafe { finish(output, LIST_OUTPUT_CAPACITY, result) }
}

/// Verifies the PDF digest and submits the document to the named Windows
/// print queue. Success means the spooler accepted the whole job, not that
/// paper was produced.
///
/// # Safety
///
/// The four string arguments must be NUL-terminated UTF-8 strings and
/// `output` must point to the 1,024-byte writable buffer declared in
/// `api.json`.
#[export_name = "RhnPrintPdf"]
pub unsafe extern "C" fn rhn_print_pdf(
    printer_name: *const u8,
    document_base64: *const u8,
    sha256: *const u8,
    copies: i32,
    job_name: *const u8,
    output: *mut u8,
) -> usize {
    let result = guarded(|| {
        let printer_name = unsafe { read_utf8(printer_name, MAX_SHORT_INPUT_BYTES) }?;
        let document_base64 = unsafe { read_utf8(document_base64, MAX_DOCUMENT_BASE64_CHARS + 1) }
            .map_err(|_| Failure::DocumentInvalid)?;
        let sha256 = unsafe { read_utf8(sha256, MAX_SHORT_INPUT_BYTES) }?;
        let job_name = unsafe { read_utf8(job_name, MAX_SHORT_INPUT_BYTES) }?;
        let request =
            validate_print_request(printer_name, document_base64, sha256, copies, job_name)?;
        platform::print_pdf(&request).map(submitted_document)
    });
    unsafe { finish(output, PRINT_OUTPUT_CAPACITY, result) }
}

fn guarded(operation: impl FnOnce() -> Result<String, Failure>) -> Result<String, Failure> {
    catch_unwind(AssertUnwindSafe(operation)).unwrap_or(Err(Failure::Internal))
}

unsafe fn finish(output: *mut u8, capacity: usize, result: Result<String, Failure>) -> usize {
    let (status, document) = match result {
        Ok(document) => (SUCCESS, document),
        Err(failure) => (failure.code(), error_document(failure)),
    };
    match unsafe { write_output(output, capacity, &document) } {
        Ok(()) => status,
        Err(failure) => {
            // The error document is always small enough for the declared buffer.
            let _ = unsafe { write_output(output, capacity, &error_document(failure)) };
            failure.code()
        }
    }
}

unsafe fn write_output(output: *mut u8, capacity: usize, value: &str) -> Result<(), Failure> {
    if output.is_null() {
        return Err(Failure::InvalidRequest);
    }
    let bytes = value.as_bytes();
    if bytes.len().saturating_add(1) > capacity {
        return Err(Failure::OutputBufferTooSmall);
    }
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), output, bytes.len());
        *output.add(bytes.len()) = 0;
    }
    Ok(())
}

/// Reads a NUL-terminated UTF-8 string of at most `limit - 1` bytes.
unsafe fn read_utf8<'a>(input: *const u8, limit: usize) -> Result<&'a str, Failure> {
    if input.is_null() {
        return Err(Failure::InvalidRequest);
    }
    for length in 0..limit {
        if unsafe { *input.add(length) } == 0 {
            return std::str::from_utf8(unsafe { std::slice::from_raw_parts(input, length) })
                .map_err(|_| Failure::InvalidRequest);
        }
    }
    Err(Failure::InvalidRequest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use webplus_plugin_config::{validate_dll_abi, ServiceDefinition};

    const SAMPLE_PDF: &[u8] = b"%PDF-1.4\n%synthetic test document\n";

    fn sample_digest() -> String {
        hex_lower(&Sha256::digest(SAMPLE_PDF))
    }

    fn sample_base64() -> String {
        BASE64.encode(SAMPLE_PDF)
    }

    #[test]
    fn valid_requests_keep_the_decoded_document_and_settings() {
        let request =
            validate_print_request("处方打印机", &sample_base64(), &sample_digest(), 2, "OUT-1")
                .unwrap();
        assert_eq!(request.document, SAMPLE_PDF);
        assert_eq!(request.copies, 2);
        assert_eq!(request.printer_name, "处方打印机");
        assert_eq!(request.job_name, "OUT-1");
    }

    #[test]
    fn digest_is_verified_before_the_document_type() {
        let other = hex_lower(&Sha256::digest(b"other"));
        assert_eq!(
            validate_print_request("P", &sample_base64(), &other, 1, "J"),
            Err(Failure::DocumentDigestMismatch)
        );
        let not_pdf = BASE64.encode(b"<html>");
        let not_pdf_digest = hex_lower(&Sha256::digest(b"<html>"));
        assert_eq!(
            validate_print_request("P", &not_pdf, &not_pdf_digest, 1, "J"),
            Err(Failure::DocumentInvalid)
        );
    }

    #[test]
    fn malformed_fields_are_rejected_without_touching_a_printer() {
        let base64 = sample_base64();
        let digest = sample_digest();
        let uppercase = digest.to_uppercase();
        let invalid_fields = [
            ("", digest.as_str(), 1, "J"),
            (" P", digest.as_str(), 1, "J"),
            ("P\n", digest.as_str(), 1, "J"),
            ("P", digest.as_str(), 0, "J"),
            ("P", digest.as_str(), 100, "J"),
            ("P", digest.as_str(), 1, ""),
            ("P", uppercase.as_str(), 1, "J"),
            ("P", "abc", 1, "J"),
        ];
        for (printer, sha256, copies, job) in invalid_fields {
            assert_eq!(
                validate_print_request(printer, &base64, sha256, copies, job),
                Err(Failure::InvalidRequest),
                "{printer:?} {sha256:?} {copies} {job:?}"
            );
        }
        for document in ["", "not base64!", "JVBERi0"] {
            assert_eq!(
                validate_print_request("P", document, &digest, 1, "J"),
                Err(Failure::DocumentInvalid),
                "{document:?}"
            );
        }
        let long_printer = "P".repeat(MAX_PRINTER_NAME_CHARS + 1);
        assert_eq!(
            validate_print_request(&long_printer, &base64, &digest, 1, "J"),
            Err(Failure::InvalidRequest)
        );
        let oversized = "A".repeat(MAX_DOCUMENT_BASE64_CHARS + 4);
        assert_eq!(
            validate_print_request("P", &oversized, &digest, 1, "J"),
            Err(Failure::DocumentInvalid)
        );
    }

    #[test]
    fn base64_limit_matches_the_decoded_document_limit() {
        assert_eq!(MAX_DOCUMENT_BASE64_CHARS, 8 * 1024 * 1024);
        assert_eq!(MAX_DOCUMENT_BASE64_CHARS / 4 * 3, MAX_DOCUMENT_BYTES);
    }

    #[test]
    fn printer_names_resolve_exactly_then_case_insensitively() {
        let printers = vec![
            Printer {
                name: "Label-01".into(),
                is_default: false,
            },
            Printer {
                name: "处方打印机".into(),
                is_default: true,
            },
        ];
        assert_eq!(
            resolve_printer(&printers, "label-01").map(|p| p.name.as_str()),
            Some("Label-01")
        );
        assert_eq!(
            resolve_printer(&printers, "处方打印机").map(|p| p.is_default),
            Some(true)
        );
        assert!(resolve_printer(&printers, "Label-02").is_none());
    }

    #[test]
    fn page_budget_bounds_pages_times_copies() {
        assert_eq!(check_page_budget(0, 1), Err(Failure::DocumentInvalid));
        assert_eq!(check_page_budget(2, 99), Ok(()));
        assert_eq!(check_page_budget(3, 67), Err(Failure::DocumentInvalid));
    }

    #[test]
    fn layout_keeps_real_size_centred_and_caps_render_resolution() {
        // A5 portrait (148 x 210 mm) on an A4 600 dpi device without margins.
        let device = DeviceGeometry {
            dpi_x: 600,
            dpi_y: 600,
            physical_width: 4960,
            physical_height: 7016,
            offset_x: 100,
            offset_y: 100,
        };
        let a5_width = 148.0 / 25.4 * 96.0;
        let a5_height = 210.0 / 25.4 * 96.0;
        let layout = layout_page(a5_width, a5_height, device).unwrap();
        assert_eq!((layout.dest_width, layout.dest_height), (3496, 4961));
        assert_eq!(layout.dest_x, (4960 - 3496) / 2 - 100);
        assert_eq!(layout.dest_y, (7016 - 4961) / 2 - 100);
        assert_eq!((layout.render_width, layout.render_height), (1748, 2480));
    }

    #[test]
    fn layout_shrinks_pages_larger_than_the_sheet() {
        let device = DeviceGeometry {
            dpi_x: 203,
            dpi_y: 203,
            physical_width: 812,
            physical_height: 406,
            offset_x: 0,
            offset_y: 0,
        };
        // An 8 x 8 inch page on a 4 x 2 inch label.
        let layout = layout_page(768.0, 768.0, device).unwrap();
        assert_eq!((layout.dest_width, layout.dest_height), (406, 406));
        assert_eq!((layout.dest_x, layout.dest_y), (203, 0));
        assert_eq!((layout.render_width, layout.render_height), (406, 406));
    }

    #[test]
    fn layout_bounds_huge_pages_and_rejects_invalid_geometry() {
        let device = DeviceGeometry {
            dpi_x: 300,
            dpi_y: 300,
            physical_width: 30_000,
            physical_height: 30_000,
            offset_x: 0,
            offset_y: 0,
        };
        let layout = layout_page(9_600.0, 9_600.0, device).unwrap();
        let pixels = f64::from(layout.render_width) * f64::from(layout.render_height);
        assert!(pixels <= MAX_RENDER_PIXELS);
        assert!(layout_page(0.0, 10.0, device).is_err());
        assert!(layout_page(10.0, 10.0, DeviceGeometry { dpi_x: 0, ..device }).is_err());
    }

    #[test]
    fn documents_match_the_published_contract() {
        let printers = [Printer {
            name: "处方\"A\"".into(),
            is_default: true,
        }];
        let listed: Value = serde_json::from_str(&printers_document(&printers)).unwrap();
        assert_eq!(
            listed,
            json!({ "printers": [{ "name": "处方\"A\"", "isDefault": true }] })
        );
        assert_eq!(
            serde_json::from_str::<Value>(&submitted_document(Some(42))).unwrap(),
            json!({ "submitted": true, "systemJobId": 42 })
        );
        assert_eq!(
            serde_json::from_str::<Value>(&submitted_document(Some(0))).unwrap(),
            json!({ "submitted": true })
        );
        assert_eq!(
            error_document(Failure::DocumentDigestMismatch),
            r#"{"error":"document-digest-mismatch"}"#
        );
    }

    #[test]
    fn stable_codes_are_unique() {
        let failures = [
            Failure::InvalidRequest,
            Failure::DocumentDigestMismatch,
            Failure::DocumentInvalid,
            Failure::PrinterNotFound,
            Failure::PrinterUnavailable,
            Failure::JobAborted,
            Failure::OutputBufferTooSmall,
            Failure::Internal,
            Failure::UnsupportedPlatform,
        ];
        let codes = failures
            .iter()
            .map(|f| f.code())
            .collect::<std::collections::BTreeSet<_>>();
        let names = failures
            .iter()
            .map(|f| f.name())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(codes.len(), failures.len());
        assert_eq!(names.len(), failures.len());
        assert!(!codes.contains(&SUCCESS));
    }

    #[test]
    fn exports_report_errors_through_the_declared_buffers() {
        let printer = b"P\0";
        let document = format!("{}\0", sample_base64());
        let wrong_digest = format!("{}\0", hex_lower(&Sha256::digest(b"other")));
        let job = b"J\0";
        let mut output = vec![0x7f_u8; PRINT_OUTPUT_CAPACITY];
        let status = unsafe {
            rhn_print_pdf(
                printer.as_ptr(),
                document.as_ptr(),
                wrong_digest.as_ptr(),
                1,
                job.as_ptr(),
                output.as_mut_ptr(),
            )
        };
        assert_eq!(status, Failure::DocumentDigestMismatch.code());
        let end = output.iter().position(|byte| *byte == 0).unwrap();
        assert_eq!(
            std::str::from_utf8(&output[..end]).unwrap(),
            r#"{"error":"document-digest-mismatch"}"#
        );

        assert_eq!(
            unsafe {
                rhn_print_pdf(
                    ptr::null(),
                    document.as_ptr(),
                    wrong_digest.as_ptr(),
                    1,
                    job.as_ptr(),
                    output.as_mut_ptr(),
                )
            },
            Failure::InvalidRequest.code()
        );
        assert_eq!(
            unsafe { rhn_list_printers(ptr::null_mut()) },
            Failure::InvalidRequest.code()
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_builds_report_unsupported_after_validation() {
        let printer = b"P\0";
        let document = format!("{}\0", sample_base64());
        let digest = format!("{}\0", sample_digest());
        let job = b"J\0";
        let mut output = vec![0_u8; PRINT_OUTPUT_CAPACITY];
        let status = unsafe {
            rhn_print_pdf(
                printer.as_ptr(),
                document.as_ptr(),
                digest.as_ptr(),
                1,
                job.as_ptr(),
                output.as_mut_ptr(),
            )
        };
        assert_eq!(status, Failure::UnsupportedPlatform.code());
        let mut list = vec![0_u8; LIST_OUTPUT_CAPACITY];
        assert_eq!(
            unsafe { rhn_list_printers(list.as_mut_ptr()) },
            Failure::UnsupportedPlatform.code()
        );
    }

    #[test]
    fn x86_and_x64_manifests_match_the_exported_abi() {
        let templates = [
            (
                "x86",
                include_str!("../../../plugins/rhn-printing/api.x86.json"),
            ),
            (
                "x64",
                include_str!("../../../plugins/rhn-printing/api.x64.json"),
            ),
        ];
        for (architecture, document) in templates {
            let services: Vec<ServiceDefinition> = serde_json::from_str(document).unwrap();
            assert_eq!(services.len(), 1);
            let service = &services[0];
            validate_dll_abi(service).unwrap();
            assert_eq!(service.service_id, "rhn.printing");
            assert_eq!(service.main_class, "bin/rhn_printing_plugin.dll");
            assert_eq!(service.resolved_main_type(), "dll");

            let raw: Value = serde_json::from_str(document).unwrap();
            assert_eq!(raw[0]["architecture"], architecture);
            assert_eq!(raw[0]["charset"], "utf8");
            let methods = raw[0]["methods"].as_array().unwrap();
            assert_eq!(methods.len(), 2);

            let list = &methods[0];
            assert_eq!(list["name"], "RhnListPrinters");
            assert_eq!(list["alias"], "listPrinters");
            assert_ne!(list["trackedInvocationRequired"], true);
            assert_eq!(
                list["parameters"],
                json!([{ "name": "$result", "type": "string", "len": LIST_OUTPUT_CAPACITY }])
            );

            let print = &methods[1];
            assert_eq!(print["name"], "RhnPrintPdf");
            assert_eq!(print["alias"], "printPdf");
            assert_eq!(print["trackedInvocationRequired"], true);
            assert_eq!(
                print["parameters"],
                json!([
                    { "name": "printerName", "type": "string" },
                    { "name": "documentBase64", "type": "string" },
                    { "name": "sha256", "type": "string" },
                    { "name": "copies", "type": "int" },
                    { "name": "jobName", "type": "string" },
                    { "name": "$result", "type": "string", "len": PRINT_OUTPUT_CAPACITY }
                ])
            );
        }
    }
}
