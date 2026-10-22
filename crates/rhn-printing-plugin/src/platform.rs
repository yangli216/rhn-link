//! Windows spooler and `Windows.Data.Pdf` implementation.
//!
//! PDF parsing and rasterisation use the system WinRT PDF renderer on a
//! dedicated MTA thread; pages are sent to the printer through a GDI
//! `WINSPOOL` device context. Nothing here runs in the desktop process: the
//! DLL is only loaded by the isolated plugin host.

use std::collections::BTreeSet;
use std::mem::size_of;

use windows::core::{Interface, PCWSTR, PWSTR};
use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
use windows::Graphics::Imaging::{
    BitmapAlphaMode, BitmapDecoder, BitmapEncoder, BitmapPixelFormat, BitmapTransform,
    ColorManagementMode, ExifOrientationMode,
};
use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};
use windows::Win32::Graphics::Gdi::{
    CreateDCW, DeleteDC, GetDeviceCaps, SetBrushOrgEx, SetStretchBltMode, StretchDIBits,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HALFTONE, HDC, LOGPIXELSX, LOGPIXELSY,
    PHYSICALHEIGHT, PHYSICALOFFSETX, PHYSICALOFFSETY, PHYSICALWIDTH, SRCCOPY,
};
use windows::Win32::Graphics::Printing::{
    EnumPrintersW, GetDefaultPrinterW, PRINTER_ENUM_CONNECTIONS, PRINTER_ENUM_LOCAL,
    PRINTER_INFO_4W,
};
use windows::Win32::Storage::Xps::{AbortDoc, EndDoc, EndPage, StartDocW, StartPage, DOCINFOW};
use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};
use windows_future::IAsyncOperation;

use crate::{
    check_page_budget, layout_page, resolve_printer, DeviceGeometry, Failure, PrintRequest, Printer,
};

const MAX_PRINTERS: usize = 512;
const ENUMERATION_ATTEMPTS: usize = 3;

pub fn list_printers() -> Result<Vec<Printer>, Failure> {
    let default_name = default_printer().map(|name| name.to_lowercase());
    let mut seen = BTreeSet::new();
    let mut printers = Vec::new();
    for name in enumerate_printer_names()? {
        if !seen.insert(name.to_lowercase()) {
            continue;
        }
        let is_default = default_name.as_deref() == Some(name.to_lowercase().as_str());
        printers.push(Printer { name, is_default });
    }
    Ok(printers)
}

pub fn print_pdf(request: &PrintRequest) -> Result<Option<i32>, Failure> {
    let printers = list_printers()?;
    let printer = resolve_printer(&printers, &request.printer_name)
        .ok_or(Failure::PrinterNotFound)?
        .name
        .clone();
    // WinRT objects are created on a fresh thread so the plugin host's
    // execution thread keeps whatever COM apartment it already uses.
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let _apartment = Apartment::multithreaded()?;
                print_on_current_thread(&printer, request)
            })
            .join()
            .unwrap_or(Err(Failure::Internal))
    })
}

fn print_on_current_thread(printer: &str, request: &PrintRequest) -> Result<Option<i32>, Failure> {
    let document = load_document(&request.document)?;
    let page_count = document.PageCount().map_err(|_| Failure::DocumentInvalid)?;
    check_page_budget(page_count, request.copies)?;

    let device = PrinterDevice::open(printer)?;
    let geometry = device.geometry()?;
    let job = device.start(&request.job_name)?;
    for _ in 0..request.copies {
        for index in 0..page_count {
            if job.print_page(&document, index, geometry).is_err() {
                job.abort();
                return Err(Failure::JobAborted);
            }
        }
    }
    job.finish()
}

fn load_document(bytes: &[u8]) -> Result<PdfDocument, Failure> {
    let load = || -> windows::core::Result<PdfDocument> {
        let stream = InMemoryRandomAccessStream::new()?;
        let writer = DataWriter::CreateDataWriter(&stream)?;
        writer.WriteBytes(bytes)?;
        writer
            .StoreAsync()?
            .cast::<IAsyncOperation<u32>>()?
            .join()?;
        writer.FlushAsync()?.join()?;
        writer.DetachStream()?;
        stream.Seek(0)?;
        PdfDocument::LoadFromStreamAsync(&stream)?.join()
    };
    load().map_err(|_| Failure::DocumentInvalid)
}

struct RenderedPage {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}

fn render_page(
    document: &PdfDocument,
    index: u32,
    width: u32,
    height: u32,
) -> windows::core::Result<RenderedPage> {
    let page = document.GetPage(index)?;
    let options = PdfPageRenderOptions::new()?;
    options.SetDestinationWidth(width)?;
    options.SetDestinationHeight(height)?;
    options.SetBitmapEncoderId(BitmapEncoder::BmpEncoderId()?)?;
    let stream = InMemoryRandomAccessStream::new()?;
    page.RenderWithOptionsToStreamAsync(&stream, &options)?
        .join()?;
    stream.Seek(0)?;
    let decoder = BitmapDecoder::CreateAsync(&stream)?.join()?;
    let width = decoder.PixelWidth()?;
    let height = decoder.PixelHeight()?;
    let provider = decoder
        .GetPixelDataTransformedAsync(
            BitmapPixelFormat::Bgra8,
            BitmapAlphaMode::Ignore,
            &BitmapTransform::new()?,
            ExifOrientationMode::IgnoreExifOrientation,
            ColorManagementMode::DoNotColorManage,
        )?
        .join()?;
    let pixels = provider.DetachPixelData()?.to_vec();
    if pixels.len() != width as usize * height as usize * 4 {
        return Err(windows::core::Error::empty());
    }
    Ok(RenderedPage {
        pixels,
        width,
        height,
    })
}

struct PrinterDevice {
    context: HDC,
}

impl PrinterDevice {
    fn open(printer: &str) -> Result<Self, Failure> {
        let driver = wide_nul("WINSPOOL");
        let name = wide_nul(printer);
        let context = unsafe {
            CreateDCW(
                PCWSTR(driver.as_ptr()),
                PCWSTR(name.as_ptr()),
                PCWSTR::null(),
                None,
            )
        };
        if context.is_invalid() {
            return Err(Failure::PrinterUnavailable);
        }
        Ok(Self { context })
    }

    fn geometry(&self) -> Result<DeviceGeometry, Failure> {
        let caps = |index| unsafe { GetDeviceCaps(Some(self.context), index) };
        let geometry = DeviceGeometry {
            dpi_x: caps(LOGPIXELSX),
            dpi_y: caps(LOGPIXELSY),
            physical_width: caps(PHYSICALWIDTH),
            physical_height: caps(PHYSICALHEIGHT),
            offset_x: caps(PHYSICALOFFSETX),
            offset_y: caps(PHYSICALOFFSETY),
        };
        if geometry.dpi_x <= 0
            || geometry.dpi_y <= 0
            || geometry.physical_width <= 0
            || geometry.physical_height <= 0
        {
            return Err(Failure::PrinterUnavailable);
        }
        Ok(geometry)
    }

    fn start(&self, job_name: &str) -> Result<PrintJob<'_>, Failure> {
        let name = wide_nul(job_name);
        let info = DOCINFOW {
            cbSize: size_of::<DOCINFOW>() as i32,
            lpszDocName: PCWSTR(name.as_ptr()),
            lpszOutput: PCWSTR::null(),
            lpszDatatype: PCWSTR::null(),
            fwType: 0,
        };
        let job_id = unsafe { StartDocW(self.context, &info) };
        if job_id <= 0 {
            return Err(Failure::PrinterUnavailable);
        }
        Ok(PrintJob {
            device: self,
            job_id,
        })
    }
}

impl Drop for PrinterDevice {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteDC(self.context);
        }
    }
}

struct PrintJob<'a> {
    device: &'a PrinterDevice,
    job_id: i32,
}

impl PrintJob<'_> {
    fn print_page(
        &self,
        document: &PdfDocument,
        index: u32,
        geometry: DeviceGeometry,
    ) -> Result<(), Failure> {
        let size = document
            .GetPage(index)
            .and_then(|page| page.Size())
            .map_err(|_| Failure::JobAborted)?;
        let layout = layout_page(f64::from(size.Width), f64::from(size.Height), geometry)?;
        // Render before StartPage so a renderer failure does not leave a
        // blank sheet in the spooled job.
        let page = render_page(document, index, layout.render_width, layout.render_height)
            .map_err(|_| Failure::JobAborted)?;
        let context = self.device.context;
        if unsafe { StartPage(context) } <= 0 {
            return Err(Failure::JobAborted);
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: page.width as i32,
                // Negative height selects a top-down DIB matching WinRT rows.
                biHeight: -(page.height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let copied = unsafe {
            SetStretchBltMode(context, HALFTONE);
            let _ = SetBrushOrgEx(context, 0, 0, None);
            StretchDIBits(
                context,
                layout.dest_x,
                layout.dest_y,
                layout.dest_width,
                layout.dest_height,
                0,
                0,
                page.width as i32,
                page.height as i32,
                Some(page.pixels.as_ptr().cast()),
                &info,
                DIB_RGB_COLORS,
                SRCCOPY,
            )
        };
        if copied <= 0 || unsafe { EndPage(context) } <= 0 {
            return Err(Failure::JobAborted);
        }
        Ok(())
    }

    fn abort(self) {
        unsafe {
            let _ = AbortDoc(self.device.context);
        }
    }

    fn finish(self) -> Result<Option<i32>, Failure> {
        if unsafe { EndDoc(self.device.context) } <= 0 {
            unsafe {
                let _ = AbortDoc(self.device.context);
            }
            return Err(Failure::JobAborted);
        }
        Ok(Some(self.job_id))
    }
}

struct Apartment;

impl Apartment {
    fn multithreaded() -> Result<Self, Failure> {
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.map_err(|_| Failure::Internal)?;
        Ok(Self)
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}

fn enumerate_printer_names() -> Result<Vec<String>, Failure> {
    let flags = PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS;
    let mut needed = 0_u32;
    let mut returned = 0_u32;
    for _ in 0..ENUMERATION_ATTEMPTS {
        // The first call reports the required size; the queue set can change
        // between calls, so the sized call is retried a bounded number of times.
        let probe =
            unsafe { EnumPrintersW(flags, PCWSTR::null(), 4, None, &mut needed, &mut returned) };
        if probe.is_ok() && needed == 0 {
            return Ok(Vec::new());
        }
        if needed == 0 {
            return Err(Failure::PrinterUnavailable);
        }
        // `usize` storage keeps PRINTER_INFO_4W pointer fields aligned.
        let mut storage = vec![0_usize; (needed as usize).div_ceil(size_of::<usize>())];
        let bytes = unsafe {
            std::slice::from_raw_parts_mut(
                storage.as_mut_ptr().cast::<u8>(),
                storage.len() * size_of::<usize>(),
            )
        };
        let mut used = 0_u32;
        if unsafe {
            EnumPrintersW(
                flags,
                PCWSTR::null(),
                4,
                Some(&mut bytes[..needed as usize]),
                &mut used,
                &mut returned,
            )
        }
        .is_err()
        {
            needed = used;
            continue;
        }
        let count = returned as usize;
        if count > MAX_PRINTERS {
            return Err(Failure::OutputBufferTooSmall);
        }
        let entries = unsafe {
            std::slice::from_raw_parts(storage.as_ptr().cast::<PRINTER_INFO_4W>(), count)
        };
        let mut names = Vec::with_capacity(count);
        for entry in entries {
            if entry.pPrinterName.is_null() {
                continue;
            }
            if let Ok(name) = unsafe { entry.pPrinterName.to_string() } {
                if !name.is_empty() {
                    names.push(name);
                }
            }
        }
        return Ok(names);
    }
    Err(Failure::PrinterUnavailable)
}

fn default_printer() -> Option<String> {
    let mut length = 0_u32;
    unsafe {
        let _ = GetDefaultPrinterW(None, &mut length);
    }
    if length == 0 || length > 4096 {
        return None;
    }
    let mut buffer = vec![0_u16; length as usize];
    if !unsafe { GetDefaultPrinterW(Some(PWSTR(buffer.as_mut_ptr())), &mut length) }.as_bool() {
        return None;
    }
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16(&buffer[..end]).ok()
}

fn wide_nul(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
