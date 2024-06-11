//! Wayland protocol implementation boilerplate.

pub mod kwin_blur;
pub mod wp_fractional_scaling;
pub mod wp_viewporter;
#[cfg(not(feature = "wayland-wl-shell"))]
pub mod xdg_activation;
pub mod qt_surface_extension;