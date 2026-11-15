//! Native HTML printing runs only in the isolated note-pdf-worker executable.
use block2::RcBlock;
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, Bool, ProtocolObject},
    sel,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSPrintInfo,
    NSPrintJobSavingURL, NSPrintOperation, NSPrintSaveJob, NSPrintingPaginationMode, NSWindow,
    NSWindowStyleMask,
};
use objc2_foundation::{
    NSDate, NSError, NSObject, NSObjectProtocol, NSPoint, NSProcessInfo, NSRect, NSRunLoop, NSSize,
    NSString, NSURL,
};
use objc2_web_kit::{WKContentWorld, WKWebView, WKWebViewConfiguration, WKWebsiteDataStore};
use std::{
    cell::RefCell,
    ffi::OsString,
    io::Read,
    path::Path,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::{Duration, Instant},
};

define_class!(
    #[unsafe(super(NSObject))]
    #[ivars = Arc<AtomicU8>]
    struct PrintCompletion;

    unsafe impl NSObjectProtocol for PrintCompletion {}

    impl PrintCompletion {
        // AppKit may invoke this on its printing thread. Only atomics cross
        // that boundary; no WebView/AppKit object is accessed by the callback.
        #[unsafe(method(printOperationDidRun:success:contextInfo:))]
        fn did_run(&self, _operation: &NSPrintOperation, success: Bool, _context: *mut std::ffi::c_void) {
            self.ivars().store(if success.as_bool() {1} else {2}, Ordering::Release);
        }
    }
);

impl PrintCompletion {
    fn new(result: Arc<AtomicU8>) -> Retained<Self> {
        let allocated = Self::alloc().set_ivars(result);
        // SAFETY: NSObject's designated initializer, with initialized Rust ivars.
        unsafe { msg_send![super(allocated), init] }
    }
}

fn pump() {
    NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.02));
}

pub fn run(arguments: Vec<OsString>) -> Result<(), String> {
    if NSProcessInfo::processInfo()
        .operatingSystemVersion()
        .majorVersion
        < 11
    {
        return Err(
            "NOTE_PDF_PLATFORM_UNAVAILABLE：原生 PDF 导出需要 macOS 11 或更高版本。".into(),
        );
    }
    if arguments.len() != 2 {
        return Err("NOTE_PDF_WORKER_INVALID：缺少隔离输入或输出路径。".into());
    }
    let input = Path::new(&arguments[0])
        .canonicalize()
        .map_err(|error| format!("NOTE_PDF_WORKER_IO：{error}"))?;
    let output = Path::new(&arguments[1]);
    if !output.is_absolute()
        || output.exists()
        || output
            .parent()
            .and_then(|path| path.canonicalize().ok())
            .as_deref()
            != input.parent()
        || input.extension().is_none_or(|value| value != "html")
        || output.extension().is_none_or(|value| value != "pdf")
    {
        return Err("NOTE_PDF_WORKER_INVALID：渲染路径不属于同一隔离目录。".into());
    }
    let parent = nix::unistd::getppid();
    let watched_output = output.to_owned();
    // The host can be terminated without running Rust destructors. Do not leave
    // a printing worker or WebKit process orphaned, even inside runOperation.
    std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(180);
        loop {
            std::thread::sleep(Duration::from_millis(250));
            if nix::unistd::getppid() != parent {
                std::process::exit(2);
            }
            if Instant::now() >= deadline {
                eprintln!("NOTE_PDF_RENDER_TIMEOUT：原生排版超时。");
                std::process::exit(3);
            }
            if std::fs::metadata(&watched_output)
                .is_ok_and(|metadata| metadata.len() > 256 * 1024 * 1024)
            {
                eprintln!("NOTE_EXPORT_TOO_LARGE：原生 PDF 超过 256 MB。");
                std::process::exit(4);
            }
        }
    });
    let main = MainThreadMarker::new().ok_or("NOTE_PDF_WORKER_THREAD：需要原生主线程。")?;
    let application = NSApplication::sharedApplication(main);
    application.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    application.finishLaunching();
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(658.0, 900.0));
    // SAFETY: This executable's main thread owns the configuration, private
    // nonpersistent WebView and hidden window until all callbacks complete.
    let (view, _window) = unsafe {
        let configuration = WKWebViewConfiguration::new(main);
        configuration.setWebsiteDataStore(&WKWebsiteDataStore::nonPersistentDataStore(main));
        let view =
            WKWebView::initWithFrame_configuration(WKWebView::alloc(main), frame, &configuration);
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(main),
            frame,
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        );
        window.setReleasedWhenClosed(false);
        window.setContentView(Some(&view));
        let input_url = NSURL::fileURLWithPath(&NSString::from_str(&input.to_string_lossy()));
        if view
            .loadFileURL_allowingReadAccessToURL(&input_url, &input_url)
            .is_none()
        {
            return Err("NOTE_PDF_DOCUMENT_UNAVAILABLE：无法打开导出 HTML。".into());
        }
        (view, window)
    };
    // No host thread is involved in this run loop. Cancellation kills/reaps
    // this child; its private output cannot become the final file on its own.
    while unsafe { view.isLoading() } {
        pump();
    }
    let ready = Rc::new(RefCell::new(None));
    let completed = ready.clone();
    let callback = RcBlock::new(move |_value: *mut AnyObject, error: *mut NSError| {
        // SAFETY: WebKit keeps NSError valid during this completion callback;
        // only its safe localized cause is retained, never script/userInfo.
        let result = if error.is_null() {
            Ok(())
        } else {
            Err(format!("NOTE_PDF_RESOURCES_FAILED：{}", unsafe {
                (&*error).localizedDescription()
            }))
        };
        *completed.borrow_mut() = Some(result);
    });
    let script = NSString::from_str(
        "if (!document.querySelector('article.note-pdf-export')) throw new Error('NOTE_PDF_DOCUMENT_UNAVAILABLE'); void document.body.offsetHeight; await document.fonts.ready; await Promise.all(Array.from(document.images, image => image.decode()));",
    );
    // SAFETY: Both frame/world and completion ABI are Apple's typed public API;
    // the callback and WebView remain retained while the worker loop is pumped.
    unsafe {
        view.callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(
            &script,
            None,
            None,
            &WKContentWorld::pageWorld(main),
            Some(&callback),
        );
    }
    while ready.borrow().is_none() {
        pump();
    }
    ready
        .borrow_mut()
        .take()
        .ok_or("NOTE_PDF_RESOURCES_FAILED：缺少资源完成回执。")??;
    let info = NSPrintInfo::new();
    info.setPaperSize(NSSize::new(595.2756, 841.8898));
    info.setLeftMargin(51.0236);
    info.setRightMargin(51.0236);
    info.setTopMargin(51.0236);
    info.setBottomMargin(51.0236);
    info.setHorizontallyCentered(false);
    info.setVerticallyCentered(false);
    info.setHorizontalPagination(NSPrintingPaginationMode::Fit);
    info.setVerticalPagination(NSPrintingPaginationMode::Automatic);
    // SAFETY: The dictionary receives the documented NSURL value for Apple's
    // save-job key. The output lives only in the validated private job directory.
    let operation = unsafe {
        info.setJobDisposition(NSPrintSaveJob);
        let url = NSURL::fileURLWithPath(&NSString::from_str(&output.to_string_lossy()));
        info.dictionary().setObject_forKey(
            (*url).as_ref(),
            ProtocolObject::from_ref(NSPrintJobSavingURL),
        );
        view.printOperationWithPrintInfo(&info)
    };
    operation.setShowsPrintPanel(false);
    operation.setShowsProgressPanel(false);
    operation.setCanSpawnSeparateThread(true);
    let result = Arc::new(AtomicU8::new(0));
    let delegate = PrintCompletion::new(Arc::clone(&result));
    // SAFETY: The selector exactly matches the BOOL/context callback above;
    // delegate, window, operation and WebView stay alive until its terminal ACK.
    // WKPrintingView requires real printing on a secondary thread. `run()` on
    // main treats its not-yet-computed page range as NSIntegerMax and can emit
    // unlimited blank pages. Use AppKit's public asynchronous print entry point.
    unsafe {
        operation.runOperationModalForWindow_delegate_didRunSelector_contextInfo(
            &_window,
            Some(&delegate),
            Some(sel!(printOperationDidRun:success:contextInfo:)),
            std::ptr::null_mut(),
        );
    }
    while result.load(Ordering::Acquire) == 0 {
        pump();
    }
    if result.load(Ordering::Acquire) != 1 {
        return Err("NOTE_PDF_RENDER_FAILED：WebKit 未完成 PDF 生成。".into());
    }
    let mut header = [0_u8; 5];
    std::fs::File::open(output)
        .and_then(|mut file| file.read_exact(&mut header))
        .map_err(|error| format!("NOTE_PDF_WORKER_IO：{error}"))?;
    if &header != b"%PDF-" {
        return Err("NOTE_PDF_RENDER_FAILED：原生输出不是有效 PDF。".into());
    }
    Ok(())
}
