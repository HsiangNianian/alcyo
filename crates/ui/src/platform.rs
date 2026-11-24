//! What differs on the iPad, kept in one place.
//!
//! The desktop takes its platform from gpui-kit; the iPad brings Zed's UIKit and Metal one itself. iOS
//! lets an app start no other program, so the board model's server (llama-server) runs inside the app there
//! (crates/llama), and other servers, as meeting search's, do not run.

/// The application, on the platform this build is for.
pub fn application() -> gpui_kit::Application {
    #[cfg(target_os = "ios")]
    {
        gpui_kit::Application::with_platform(gpui_ios::current_platform(false))
    }
    #[cfg(not(target_os = "ios"))]
    {
        gpui_kit::application()
    }
}

/// Whether the app may start programs of its own, as the meeting search model's server is: not on the iPad.
pub const OWN_PROGRAMS: bool = !cfg!(target_os = "ios");

/// Whether a board model can run on this device. Always on a desktop, where it is a program of its own. On the iPad
/// it runs inside the app (alcyo-llama), on one with memory enough for the 4B model: 8 GB, as an iPad Air with M2
/// has. iPadOS lets an app use only part of it, so starting the model checks again what is left.
pub fn local_models() -> bool {
    #[cfg(target_os = "ios")]
    {
        const ENOUGH: u64 = 7 << 30;
        alcyo_llama::BUILT_IN && alcyo_llama::physical_memory().is_some_and(|bytes| bytes >= ENOUGH)
    }
    #[cfg(not(target_os = "ios"))]
    {
        true
    }
}

/// Whether the window is laid out for the iPad: the board over the whole screen, with everything else
/// floating over it (crates/ui/src/ipad.rs), rather than the desktop's title bar, dock and side panel.
///
/// Always on the iPad. As a development aid, a desktop build takes it too when started with
/// `ALCYO_IPAD_LAYOUT=1`, so the iPad's screen can be worked on and screenshotted at its size without one.
pub fn ipad_layout() -> bool {
    if cfg!(target_os = "ios") {
        return true;
    }
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("ALCYO_IPAD_LAYOUT").is_ok_and(|v| v == "1"))
}

/// What the system draws over the window, for the iPad layout to keep clear of: the status bar, the home
/// indicator and a rounded screen's corners. A desktop build laid out for the iPad gets an iPad's landscape
/// insets (a 24 pt status bar, a 20 pt home indicator), so it lines up with the iPad's screen.
pub fn safe_area(window: &gpui_kit::Window) -> gpui_kit::Edges<gpui_kit::Pixels> {
    if cfg!(target_os = "ios") {
        window.insets().safe_area
    } else {
        gpui_kit::Edges { top: gpui_kit::px(24.), bottom: gpui_kit::px(20.), left: gpui_kit::px(0.), right: gpui_kit::px(0.) }
    }
}

/// Hands a file the app has just written to the share sheet (Save to Files, AirDrop, Mail…). The iPad has no
/// save dialog: `cx.prompt_for_new_path` answers there with a temporary path, and once the file is written
/// there this offers it. A desktop build saved it where the person chose, so it does nothing.
#[allow(dead_code)]
pub fn share_file(path: &std::path::Path) {
    #[cfg(target_os = "ios")]
    if let Err(error) = gpui_ios::share_file(path) {
        eprintln!("could not share {}: {error:#}", path.display());
    }
    #[cfg(not(target_os = "ios"))]
    let _ = path;
}

/// How far from the left a bar along the top must start, at least `wanted`, to keep clear of iPadOS's window
/// controls in a windowed scene when they are up there with it (`top` to `bottom` of the bar).
pub fn clear_of_controls(window: &gpui_kit::Window, wanted: f32, top: f32, bottom: f32) -> f32 {
    match window_controls(window) {
        Some(r) if f32::from(r.origin.y) < bottom && f32::from(r.origin.y + r.size.height) > top && f32::from(r.origin.x) < wanted + 80.0 => wanted.max(f32::from(r.origin.x + r.size.width) + 8.0),
        _ => wanted,
    }
}

/// Where iPadOS draws a windowed scene's controls (the three dots, top left): a corner of the window to keep
/// clear of, besides the safe area. `None` in full screen, before iPadOS 26, and on the desktop.
#[allow(dead_code)]
pub fn window_controls(window: &gpui_kit::Window) -> Option<gpui_kit::Bounds<gpui_kit::Pixels>> {
    let _ = window;
    #[cfg(target_os = "ios")]
    {
        gpui_ios::window_controls_rect()
    }
    #[cfg(not(target_os = "ios"))]
    {
        None
    }
}

/// Calls `handler` with the number of fingers (2 or 3) when the iPad sees a two- or three-finger tap, for undo
/// and redo. The tap's touches are cancelled, so it is not also a click. A desktop build never calls it.
#[allow(dead_code)]
pub fn on_multi_finger_tap(cx: &mut gpui_kit::App, handler: impl FnMut(u8, &mut gpui_kit::App) + 'static) {
    #[cfg(target_os = "ios")]
    {
        let cx = cx.to_async();
        let handler = std::rc::Rc::new(std::cell::RefCell::new(handler));
        // UIKit calls back from its own run loop; handle the tap once that has unwound.
        gpui_ios::on_multi_finger_tap(move |fingers| {
            let handler = handler.clone();
            cx.spawn(async move |cx| cx.update(|cx| (handler.borrow_mut())(fingers, cx))).detach();
        });
    }
    #[cfg(not(target_os = "ios"))]
    let _ = (cx, handler);
}

/// Something done on the Apple Pencil itself.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PencilAction {
    /// A double tap on its side (Pencil 2 and Pro).
    DoubleTap,
    /// A squeeze (Pencil Pro, iPadOS 17.5 and later).
    Squeeze,
}

/// Calls `handler` when the Pencil is double-tapped or squeezed. A desktop build never calls it.
#[allow(dead_code)]
pub fn on_pencil_action(cx: &mut gpui_kit::App, handler: impl FnMut(PencilAction, &mut gpui_kit::App) + 'static) {
    #[cfg(target_os = "ios")]
    {
        let cx = cx.to_async();
        let handler = std::rc::Rc::new(std::cell::RefCell::new(handler));
        // UIKit calls back from its own run loop; handle the action once that has unwound.
        gpui_ios::on_pencil_action(move |action| {
            let action = match action {
                gpui_ios::PencilAction::DoubleTap => PencilAction::DoubleTap,
                gpui_ios::PencilAction::Squeeze => PencilAction::Squeeze,
            };
            let handler = handler.clone();
            cx.spawn(async move |cx| cx.update(|cx| (handler.borrow_mut())(action, cx))).detach();
        });
    }
    #[cfg(not(target_os = "ios"))]
    let _ = (cx, handler);
}

/// Whether the touch being handled comes from the Apple Pencil rather than a finger. Read it in a touch or
/// pointer handler, while the event is dispatched: it describes the touch being delivered.
///
/// On the iPad it is the touch's own type. On a desktop the mouse is what draws, so it counts as the Pencil,
/// unless started with `ALCYO_PENCIL_MOUSE=0`, a development aid that takes the mouse for a finger.
pub fn touch_is_pencil() -> bool {
    #[cfg(target_os = "ios")]
    {
        gpui_ios::current_touch_is_pencil()
    }
    #[cfg(not(target_os = "ios"))]
    {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| std::env::var("ALCYO_PENCIL_MOUSE").map_or(true, |v| v != "0"))
    }
}

/// A second screen the room watches: on the iPad, a display on a cable or AirPlay.
#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)]
pub struct RoomDisplay {
    pub id: gpui_kit::DisplayId,
    /// What to call it; UIKit tells no more than that it is external.
    pub name: String,
    /// Its size in points.
    pub size: gpui_kit::Size<gpui_kit::Pixels>,
}

/// The room's display, if one is connected. Its coming and going is reported by `cx.observe_displays`.
///
/// A desktop build has none, unless started with `ALCYO_ROOM_DISPLAY=1`, a development aid that offers an
/// ordinary 1280×720 window in its place.
#[allow(dead_code)]
pub fn room_display(cx: &gpui_kit::App) -> Option<RoomDisplay> {
    #[cfg(target_os = "ios")]
    {
        let _ = cx;
        gpui_ios::external_display().map(|display| RoomDisplay {
            id: display.id,
            name: "External display".into(),
            size: display.size,
        })
    }
    #[cfg(not(target_os = "ios"))]
    {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if !*ON.get_or_init(|| std::env::var("ALCYO_ROOM_DISPLAY").is_ok_and(|v| v == "1")) {
            return None;
        }
        Some(RoomDisplay {
            id: cx.primary_display().map_or(gpui_kit::DisplayId::new(0), |display| display.id()),
            name: "Room display (development)".into(),
            size: gpui_kit::size(gpui_kit::px(1280.), gpui_kit::px(720.)),
        })
    }
}

/// Opens a window on the room's display showing the view `build` makes. On the iPad it fills the display,
/// takes no input, and closes by itself when the display goes away.
#[allow(dead_code)]
pub fn open_room_window<V: gpui_kit::Render>(
    cx: &mut gpui_kit::App,
    display: &RoomDisplay,
    build: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> gpui_kit::Entity<V>,
) -> anyhow::Result<(gpui_kit::AnyWindowHandle, gpui_kit::Entity<V>)> {
    #[cfg(target_os = "ios")]
    let options = gpui_kit::WindowOptions { display_id: Some(display.id), ..Default::default() };
    #[cfg(not(target_os = "ios"))]
    let options = {
        let id = cx.find_display(display.id).map(|found| found.id());
        gpui_kit::WindowOptions {
            display_id: id,
            window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds::centered(id, display.size, cx))),
            titlebar: Some(gpui_kit::TitlebarOptions { title: Some(display.name.clone().into()), ..Default::default() }),
            focus: false,
            ..Default::default()
        }
    };
    gpui_kit::open_window(options, cx, build)
}
