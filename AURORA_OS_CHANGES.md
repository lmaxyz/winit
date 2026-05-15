# Aurora OS Changes Summary

This document describes the main changes made to this winit fork for **Aurora OS** compatibility.

## What is Winit?

**Winit** is the standard Rust library for cross-platform window creation and event-loop management. This fork is based on **winit 0.30.13** and adapts it for **Aurora OS** — a Sailfish-derived Linux mobile OS that uses the **Lipstick** compositor.

---

## Main Changes

The changes are concentrated in `src/platform_impl/linux/wayland/`.

### 1. Shell Protocol: `xdg_shell` → `wl_shell`

The biggest architectural change is replacing the modern `xdg_shell` with the legacy **`wl_shell`** protocol, required by the Lipstick compositor.

**New files:**
- `src/platform_impl/linux/wayland/shell/wl_shell/mod.rs` — binds `wl_shell` global
- `src/platform_impl/linux/wayland/shell/wl_shell/window.rs` — `WlShellWindow` wrapper

**Impact:** Many desktop window features are disabled or stubbed:
- `xdg_toplevel()` returns `None`
- Resizing, min/max inner size, resizability → no-ops
- Decorations are handled server-side by Lipstick
- Minimize is ignored

### 2. Qt Extended Surface (`qt_surface_extension`)

Aurora OS uses a QtWayland-specific protocol for window lifecycle and property passing.

**New file:**
- `src/platform_impl/linux/wayland/types/qt_surface_extension.rs`

**Handles:**
- `Close` events → `WindowEvent::CloseRequested`
- `OnscreenVisibility` → focus changes
- `SetGenericProperty` → generic compositor properties
- **Sending** properties via `update_generic_property(name, value)`

### 3. New Public API: `Window::update_generic_property`

Exposed all the way to the public API in `src/window.rs`:

```rust
impl Window {
    pub fn update_generic_property(&self, name: &str, value: Vec<u8>) { ... }
}
```

This allows Aurora apps to send arbitrary key-value properties to the Lipstick compositor (e.g., for cover page configuration or window behavior hints).

### 4. Orientation Handling

On transform changes, the window forces `content_orientation_mask` to `LandscapeOrientation` via the Qt extended surface.

### 5. Maliit IME Removal

The branch name (`rm_maliit`) and commented-out code in `window/state.rs` suggest **Maliit** (virtual keyboard IME) integration was removed or is being removed.

---

## Other Platforms

- **X11**: Only has a stub `update_generic_property` no-op to satisfy the Linux `Window` enum.
- **Windows, macOS, iOS, Android, Web**: Completely unchanged.

---

## Quick Reference: Key Aurora-Specific Files

| File | Purpose |
|------|---------|
| `src/platform_impl/linux/wayland/shell/wl_shell/mod.rs` | `wl_shell` global binding |
| `src/platform_impl/linux/wayland/shell/wl_shell/window.rs` | `WlShellWindow` surface wrapper |
| `src/platform_impl/linux/wayland/types/qt_surface_extension.rs` | QtExtendedSurface protocol |
| `src/platform_impl/linux/wayland/window/state.rs` | Stores `extended_surface`, handles properties |
| `src/window.rs` | Public `update_generic_property` API |

---

## Summary

This fork trades modern desktop Wayland features for compatibility with Aurora OS's legacy `wl_shell`-based mobile compositor, while adding Qt-specific protocols for window lifecycle and generic property communication.
