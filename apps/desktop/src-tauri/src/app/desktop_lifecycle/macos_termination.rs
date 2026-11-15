//! AppKit termination does not pass through Tao's preventable ExitRequested.
//! Cancel it synchronously; the existing async coordinator confirms AppHandle::exit.
use super::APP_QUIT_REQUEST_EVENT;
use objc2::{
    Encode, MainThreadMarker, ffi,
    runtime::{AnyClass, AnyObject, Imp, Sel},
    sel,
};
use objc2_app_kit::{NSApplication, NSApplicationTerminateReply};
use std::{ffi::CString, io, sync::OnceLock};
use tauri::{AppHandle, Emitter};

// AppKit's delegate callback cannot carry Rust state. The application owns this
// handle for its process lifetime, and setup installs the callback only once.
static QUIT_APP: OnceLock<AppHandle> = OnceLock::new();

pub fn install_termination_guard(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let main = MainThreadMarker::new()
        .ok_or_else(|| io::Error::other("AppKit quit guard requires the main thread"))?;
    let application = NSApplication::sharedApplication(main);
    let delegate = application
        .delegate()
        .ok_or_else(|| io::Error::other("Missing AppKit application delegate"))?;
    let delegate_object: &AnyObject = (*delegate).as_ref();
    let class = delegate_object.class();
    // The pinned Tao 0.35.3 delegate owns the existing callbacks and ivars. Only
    // add the missing method; never replace the delegate or swizzle its methods.
    if class.name() != c"TaoAppDelegateParent" {
        return Err(io::Error::other("Unsupported AppKit application delegate").into());
    }
    if class
        .instance_method(sel!(applicationShouldTerminate:))
        .is_some()
    {
        return Err(io::Error::other("AppKit termination handler already exists").into());
    }
    QUIT_APP
        .set(app.clone())
        .map_err(|_| io::Error::other("AppKit quit guard already installed"))?;
    add_termination_method(class)?;
    Ok(())
}

fn add_termination_method(class: &AnyClass) -> Result<(), io::Error> {
    let selector = sel!(applicationShouldTerminate:);
    if class.instance_method(selector).is_some() {
        return Err(io::Error::other(
            "AppKit termination handler already exists",
        ));
    }
    let encoding = CString::new(format!("{}@:@", NSApplicationTerminateReply::ENCODING))
        .map_err(|_| io::Error::other("Invalid AppKit termination encoding"))?;
    // SAFETY: AppKit calls this selector with (id receiver, SEL, id sender) and
    // an NSUInteger return. The typed callback returns the repr-transparent
    // NSApplicationTerminateReply; its encoding comes from the locked bindings.
    // IMP is an erased function pointer, never invoked with the erased signature.
    let implementation: Imp = unsafe {
        std::mem::transmute::<
            extern "C-unwind" fn(
                *mut AnyObject,
                Sel,
                *mut AnyObject,
            ) -> NSApplicationTerminateReply,
            Imp,
        >(should_terminate)
    };
    // SAFETY: Objective-C owns this registered class for the process lifetime.
    // class_addMethod mutates its runtime metadata through the public C API,
    // preserving its layout and all existing methods. The selector/signature
    // match the callback above; the C string remains alive through registration.
    let added = unsafe {
        ffi::class_addMethod(
            (class as *const AnyClass).cast_mut(),
            selector,
            implementation,
            encoding.as_ptr(),
        )
    };
    if !added.as_bool() {
        return Err(io::Error::other("Could not install AppKit quit guard"));
    }
    Ok(())
}

extern "C-unwind" fn should_terminate(
    _receiver: *mut AnyObject,
    _selector: Sel,
    _sender: *mut AnyObject,
) -> NSApplicationTerminateReply {
    if let Some(app) = QUIT_APP.get()
        && let Err(error) = app.emit(APP_QUIT_REQUEST_EVENT, ())
    {
        eprintln!("Could not request safe AppKit quit: {error}");
    }
    // Keep the regular event loop active for WebView IPC and async persistence.
    // Failed or undelivered requests remain canceled and can be retried.
    NSApplicationTerminateReply::TerminateCancel
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2::{msg_send, rc::Retained, runtime::ClassBuilder};

    #[test]
    fn registered_selector_cancels_and_preserves_existing_delegate_methods() {
        let builder =
            ClassBuilder::new(c"ResearchAssistantQuitGuardTest", objc2::class!(NSObject)).unwrap();
        let class = builder.register();
        add_termination_method(class).unwrap();
        assert!(add_termination_method(class).is_err());
        assert!(class.instance_method(sel!(description)).is_some());
        // SAFETY: This test class inherits NSObject's new, whose owned result
        // has no custom ivars. The registered selector has the exact AppKit ABI;
        // its sender is opaque and never dereferenced by the callback.
        unsafe {
            let object: Retained<AnyObject> = msg_send![class, new];
            let reply: NSApplicationTerminateReply = msg_send![&*object,
                applicationShouldTerminate: std::ptr::null_mut::<AnyObject>()];
            assert_eq!(reply, NSApplicationTerminateReply::TerminateCancel);
        }
    }
}
