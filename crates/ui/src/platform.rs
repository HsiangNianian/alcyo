//! What differs on the iPad, kept in one place.
//!
//! The desktop takes its platform from gpui-kit; the iPad brings Zed's UIKit and Metal one itself. iOS
//! lets an app start no other program, so the board model's server (llama-server) cannot run there:
//! the iPad draws with a remote model until llama.cpp runs inside the app.

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

/// Whether a board model can run on this device, which needs a program of its own.
pub const LOCAL_MODELS: bool = !cfg!(target_os = "ios");

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
