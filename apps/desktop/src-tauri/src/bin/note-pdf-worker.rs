//! A disposable native renderer process. AppKit/WebKit never run on the host
//! application's UI thread; the host owns cancellation and atomic publication.
#[cfg(target_os = "macos")]
#[path = "../core/note_export/native_pdf.rs"]
mod native_pdf;

fn main() {
    #[cfg(target_os = "macos")]
    let result =
        objc2::rc::autoreleasepool(|_| native_pdf::run(std::env::args_os().skip(1).collect()));
    #[cfg(not(target_os = "macos"))]
    let result: Result<(), String> =
        Err("NOTE_PDF_PLATFORM_UNAVAILABLE：此平台尚无原生 PDF 导出。".into());
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
