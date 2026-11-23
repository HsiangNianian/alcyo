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
