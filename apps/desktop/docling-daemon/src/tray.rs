//! macOS menu bar owned by the daemon itself; the server runs on a worker thread.
use crate::control::{Control, Status};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tao::{
    event::{Event, StartCause},
    event_loop::{ControlFlow, EventLoopBuilder},
    platform::{
        macos::{ActivationPolicy, EventLoopExtMacOS},
        run_return::EventLoopExtRunReturn,
    },
};
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

#[derive(Debug)]
enum UiEvent {
    Menu(MenuEvent),
    Finished,
}

struct MenuBar {
    _icon: TrayIcon,
    status: MenuItem,
    clients: MenuItem,
    address: MenuItem,
    quit: MenuItem,
    previous: Option<Status>,
}

impl MenuBar {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let menu = Menu::new();
        let title = MenuItem::new("Docling 服务", false, None);
        let status = MenuItem::new("状态：正在启动／加载模型", false, None);
        let clients = MenuItem::new("客户端：0 · 解析任务：0", false, None);
        let address = MenuItem::new("地址：正在准备", false, None);
        let quit = MenuItem::new("退出 Docling", true, None);
        let separator = PredefinedMenuItem::separator();
        menu.append_items(&[&title, &status, &clients, &address, &separator, &quit])?;
        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Docling 服务 · 正在启动")
            .with_icon(application_icon()?)
            .with_icon_as_template(true)
            .build()?;
        Ok(Self {
            _icon: icon,
            status,
            clients,
            address,
            quit,
            previous: None,
        })
    }

    fn update(&mut self, status: Status) {
        if self.previous.as_ref() == Some(&status) {
            return;
        }
        self.status.set_text(status.label());
        self.clients.set_text(format!(
            "客户端：{} · 解析任务：{}",
            status.clients, status.requests
        ));
        self.address.set_text(
            status
                .port
                .map(|port| format!("地址：127.0.0.1:{port}"))
                .unwrap_or_else(|| "地址：正在准备".into()),
        );
        self.quit.set_text(if status.stopping {
            "正在退出 Docling…"
        } else {
            "退出 Docling"
        });
        self.quit.set_enabled(!status.stopping);
        let _ = self
            ._icon
            .set_tooltip(Some(format!("Docling 服务 · {}", status.label())));
        self.previous = Some(status);
    }

    fn quit(&self, control: &Control) {
        let status = control.status();
        if status.stopping {
            return;
        }
        if status.requests > 0 {
            let answer = rfd::MessageDialog::new()
                .set_title("退出 Docling？")
                .set_description(format!("当前有 {} 个解析任务。退出会中断所有客户端的解析；当前页面处理结束后退出，历史解析结果保留。", status.requests))
                .set_level(rfd::MessageLevel::Warning)
                .set_buttons(rfd::MessageButtons::OkCancelCustom("退出 Docling".into(), "继续运行".into()))
                .show();
            if answer != rfd::MessageDialogResult::Custom("退出 Docling".into()) {
                return;
            }
        }
        control.request_stop();
    }
}

// The approved seal uses real alpha cutouts so macOS can tint the template.
fn application_icon() -> Result<Icon, Box<dyn std::error::Error>> {
    let image = image::load_from_memory_with_format(
        include_bytes!("../assets/menu-bar-logo.png"),
        image::ImageFormat::Png,
    )?
    .into_rgba8();
    let (width, height) = image.dimensions();
    Ok(Icon::from_rgba(image.into_raw(), width, height)?)
}

pub(crate) fn run() -> Result<(), Box<dyn std::error::Error>> {
    let control = Control::default();
    let result = Arc::new(Mutex::new(None));
    let mut event_loop = EventLoopBuilder::<UiEvent>::with_user_event().build();
    event_loop.set_activation_policy(ActivationPolicy::Accessory);
    event_loop.set_dock_visibility(false);
    event_loop.set_activate_ignoring_other_apps(false);
    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UiEvent::Menu(event));
    }));
    let mut menu: Option<MenuBar> = None;
    let mut worker = None;
    let mut ui_error = None;
    let worker_proxy = event_loop.create_proxy();
    event_loop.run_return(|event, _, flow| {
        *flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(250));
        match event {
            Event::NewEvents(StartCause::Init) => {
                match MenuBar::new() {
                    Ok(bar) => menu = Some(bar),
                    Err(error) => {
                        ui_error = Some(error.to_string());
                        *flow = ControlFlow::Exit;
                        return;
                    }
                }
                let service_control = control.clone();
                let finished = result.clone();
                let proxy = worker_proxy.clone();
                worker = match std::thread::Builder::new()
                    .name("docling-server".into())
                    .spawn(move || {
                        let outcome =
                            std::panic::catch_unwind(|| crate::run_runtime(service_control))
                                .map_err(|_| {
                                    "[server_panicked] Docling 服务线程异常退出".to_owned()
                                })
                                .and_then(|result| {
                                    result.map_err(|error| format!("[server_failed] {error}"))
                                });
                        if let Ok(mut result) = finished.lock() {
                            *result = Some(outcome);
                        }
                        let _ = proxy.send_event(UiEvent::Finished);
                    }) {
                    Ok(worker) => Some(worker),
                    Err(error) => {
                        ui_error = Some(error.to_string());
                        *flow = ControlFlow::Exit;
                        None
                    }
                };
            }
            Event::UserEvent(UiEvent::Menu(event)) => {
                if let Some(menu) = &menu
                    && event.id() == menu.quit.id()
                {
                    menu.quit(&control);
                }
            }
            Event::UserEvent(UiEvent::Finished) => {
                *flow = ControlFlow::Exit;
            }
            Event::MainEventsCleared => {
                if let Some(menu) = &mut menu {
                    menu.update(control.status());
                }
            }
            _ => {}
        }
    });
    drop(menu); // The icon is released by the same process that owned inference.
    if let Some(worker) = worker {
        let _ = worker.join();
    }
    let outcome = result
        .lock()
        .map_err(|_| "[state_unavailable] Docling 退出状态不可用")?
        .take();
    if let Some(error) = ui_error.or_else(|| outcome.and_then(Result::err)) {
        return Err(std::io::Error::other(error).into());
    }
    Ok(())
}
