//! The state of the window, which is shared with the event-loop.

use std::num::NonZeroU32;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use log::{info, warn};

use sctk::reexports::client::protocol::wl_seat::WlSeat;
use sctk::reexports::client::protocol::wl_shm::WlShm;
use sctk::reexports::client::protocol::wl_surface::WlSurface;
use sctk::reexports::client::{Connection, Proxy, QueueHandle};
use sctk::reexports::csd_frame::{FrameAction, ResizeEdge, DecorationsFrame, FrameClick};
use sctk::reexports::protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::WpFractionalScaleV1;
use sctk::reexports::protocols::wp::text_input::zv3::client::zwp_text_input_v3::ZwpTextInputV3;
use sctk::reexports::protocols::wp::viewporter::client::wp_viewport::WpViewport;

use sctk::compositor::{CompositorState, Region};
use sctk::seat::pointer::ThemedPointer;
use sctk::shell::xdg::window::WindowConfigure;
use sctk::shell::WaylandSurface;
use sctk::shm::slot::SlotPool;
use sctk::shm::Shm;
use sctk::subcompositor::SubcompositorState;
use wayland_client::protocol::wl_shell_surface::Resize;
use wayland_protocols_plasma::blur::client::org_kde_kwin_blur::OrgKdeKwinBlur;
use wayland_protocols_plasma::surface_extension::client::qt_extended_surface::QtExtendedSurface;

use crate::dpi::{LogicalPosition, LogicalSize, PhysicalSize, Size};
use crate::error::{ExternalError, NotSupportedError};
// use crate::event::WindowEvent;
use crate::platform_impl::wayland::shell::wl_shell::window::Window as WlShellWindow;
use crate::platform_impl::wayland::event_loop::sink::EventSink;
// use crate::platform_impl::wayland::make_wid;
use crate::platform_impl::wayland::types::kwin_blur::KWinBlurManager;
use crate::platform_impl::WindowId;
use crate::window::{CursorGrabMode, CursorIcon, ImePurpose, ResizeDirection, Theme};

use crate::platform_impl::wayland::seat::{
    PointerConstraintsState, WinitPointerData, WinitPointerDataExt, ZwpTextInputV3Ext,
};
use crate::platform_impl::wayland::state::{WindowCompositorUpdate, WinitState};

#[cfg(feature = "sctk-adwaita")]
pub type WinitFrame = sctk_adwaita::AdwaitaFrame<WinitState>;

// Minimum window inner size.
const MIN_WINDOW_SIZE: LogicalSize<u32> = LogicalSize::new(2, 1);

/// The state of the window which is being updated from the [`WinitState`].
pub struct WindowState {
    /// The connection to Wayland server.
    pub connection: Connection,

    /// The window frame, which is created from the configure request.
    frame: Option<WinitFrame>,

    /// The `Shm` to set cursor.
    pub shm: WlShm,

    // A shared pool where to allocate custom cursors.
    _custom_cursor_pool: Arc<Mutex<SlotPool>>,

    /// The last received configure.
    pub last_configure: Option<WindowConfigure>,

    /// The pointers observed on the window.
    pub pointers: Vec<Weak<ThemedPointer<WinitPointerData>>>,

    /// Cursor icon.
    pub cursor_icon: CursorIcon,

    /// Wether the cursor is visible.
    pub cursor_visible: bool,

    /// Pointer constraints to lock/confine pointer.
    pub pointer_constraints: Option<Arc<PointerConstraintsState>>,

    /// Queue handle.
    pub queue_handle: QueueHandle<WinitState>,

    /// Theme varaint.
    theme: Option<Theme>,

    /// The current window title.
    title: String,

    /// Whether the frame is resizable.
    resizable: bool,

    /// Whether the window has focus.
    has_focus: bool,

    /// The scale factor of the window.
    scale_factor: f64,

    /// Whether the window is transparent.
    transparent: bool,

    /// The state of the compositor to create WlRegions.
    compositor: Arc<CompositorState>,

    /// The current cursor grabbing mode.
    cursor_grab_mode: GrabState,

    /// Whether the IME input is allowed for that window.
    ime_allowed: bool,

    /// The current IME purpose.
    ime_purpose: ImePurpose,

    /// The text inputs observed on the window.
    text_inputs: Vec<ZwpTextInputV3>,

    /// The inner size of the window, as in without client side decorations.
    size: LogicalSize<u32>,

    /// Whether the CSD fail to create, so we don't try to create them on each iteration.
    _csd_fails: bool,

    /// Whether we should decorate the frame.
    decorate: bool,

    /// Min size.
    min_inner_size: LogicalSize<u32>,
    max_inner_size: Option<LogicalSize<u32>>,

    /// The size of the window when no states were applied to it. The primary use for it
    /// is to fallback to original window size, before it was maximized, if the compositor
    /// sends `None` for the new size in the configure.
    stateless_size: LogicalSize<u32>,

    /// Initial window size provided by the user. Removed on the first
    /// configure.
    initial_size: Option<Size>,

    /// The state of the frame callback.
    frame_callback_state: FrameCallbackState,

    viewport: Option<WpViewport>,
    fractional_scale: Option<WpFractionalScaleV1>,
    blur: Option<OrgKdeKwinBlur>,
    blur_manager: Option<KWinBlurManager>,

    /// Whether the client side decorations have pending move operations.
    ///
    /// The value is the serial of the event triggered moved.
    has_pending_move: Option<u32>,

    /// The underlying SCTK window.
    pub wl_window: WlShellWindow,

    // QtExtendedSurface global, provides close event
    _extended_surface: Option<QtExtendedSurface>
}

impl WindowState {
    /// Create new window state.
    pub fn new(
        connection: Connection,
        queue_handle: &QueueHandle<WinitState>,
        winit_state: &WinitState,
        initial_size: Size,
        wl_window: WlShellWindow,
        theme: Option<Theme>,
    ) -> Self {
        let compositor = winit_state.compositor_state.clone();
        let pointer_constraints = winit_state.pointer_constraints.clone();

        let wl_surface = wl_window.wl_surface();

        let viewport = winit_state
            .viewporter_state
            .as_ref()
            .map(|state| {
                state.get_viewport(wl_surface, queue_handle)
            });

        let fractional_scale = winit_state
            .fractional_scaling_manager
            .as_ref()
            .map(|fsm| {
                fsm.fractional_scaling(wl_surface, queue_handle)
            });

        let extended_surface = winit_state.surface_extension.as_ref().map(|se| se.get_extended_surface(wl_surface, &queue_handle));

        Self {
            blur: None,
            blur_manager: winit_state.kwin_blur_manager.clone(),
            _extended_surface: extended_surface,
            compositor,
            connection,
            _csd_fails: false,
            cursor_grab_mode: GrabState::new(),
            cursor_icon: CursorIcon::Default,
            cursor_visible: true,
            decorate: true,
            fractional_scale,
            frame: None,
            frame_callback_state: FrameCallbackState::None,
            has_focus: false,
            has_pending_move: None,
            ime_allowed: false,
            ime_purpose: ImePurpose::Normal,
            last_configure: None,
            max_inner_size: None,
            min_inner_size: MIN_WINDOW_SIZE,
            pointer_constraints,
            pointers: Default::default(),
            queue_handle: queue_handle.clone(),
            resizable: true,
            scale_factor: 1.,
            shm: winit_state.shm.wl_shm().clone(),
            _custom_cursor_pool: winit_state.custom_cursor_pool.clone(),
            size: initial_size.to_logical(1.),
            stateless_size: initial_size.to_logical(1.),
            initial_size: Some(initial_size),
            text_inputs: Vec::new(),
            theme,
            title: String::default(),
            transparent: false,
            viewport,
            wl_window
        }
    }

    /// Apply closure on the given pointer.
    fn apply_on_poiner<F: Fn(&ThemedPointer<WinitPointerData>, &WinitPointerData)>(
        &self,
        callback: F,
    ) {
        self.pointers
            .iter()
            .filter_map(Weak::upgrade)
            .for_each(|pointer| {
                let data = pointer.pointer().winit_data();
                callback(pointer.as_ref(), data);
            })
    }

    /// Get the current state of the frame callback.
    pub fn frame_callback_state(&self) -> FrameCallbackState {
        self.frame_callback_state
    }

    /// The frame callback was received, but not yet sent to the user.
    pub fn frame_callback_received(&mut self) {
        self.frame_callback_state = FrameCallbackState::Received;
    }

    /// Reset the frame callbacks state.
    pub fn frame_callback_reset(&mut self) {
        self.frame_callback_state = FrameCallbackState::None;
    }

    /// Request a frame callback if we don't have one for this window in flight.
    pub fn request_frame_callback(&mut self) {
        let surface = self.wl_window.wl_surface();
        match self.frame_callback_state {
            FrameCallbackState::None | FrameCallbackState::Received => {
                self.frame_callback_state = FrameCallbackState::Requested;
                surface.frame(&self.queue_handle, surface.clone());
            }
            FrameCallbackState::Requested => (),
        }
    }

    pub fn configure(
        &mut self,
        configure: WindowConfigure,
        _shm: &Shm,
        _compositor: &Arc<CompositorState>,
        _subcompositor: &Option<Arc<SubcompositorState>>,
        _event_sink: &mut EventSink,
    ) -> LogicalSize<u32> {
        // NOTE: when using fractional scaling or wl_compositor@v6 the scaling
        // should be delivered before the first configure, thus apply it to
        // properly scale the physical sizes provided by the users.
        if let Some(initial_size) = self.initial_size.take() {
            self.size = initial_size.to_logical(self.scale_factor());
            self.stateless_size = self.size;
        }

        let stateless = Self::is_stateless(&configure);

        // Emit `Occluded` event on suspension change.
        // ToDo: Make occluded by OnscreenVisibility event of qt_extended_surface
        // let occluded = configure.state.contains(XdgWindowState::SUSPENDED);
        // if self
        //     .last_configure
        //     .as_ref()
        //     .map(|c| c.state.contains(XdgWindowState::SUSPENDED))
        //     .unwrap_or(false)
        //     != occluded
        // {   
        //     let wl_surface = self.wl_window.wl_surface();

        //     let window_id = make_wid(wl_surface);
        //     event_sink.push_window_event(WindowEvent::Occluded(occluded), window_id);
        // }

        let (mut new_size, constrain) = if let Some(frame) = self.frame.as_mut() {
            // Configure the window states.
            frame.update_state(configure.state);

            match configure.new_size {
                (Some(width), Some(height)) => {
                    let (width, height) = frame.subtract_borders(width, height);
                    (
                        (
                            width.map(|w| w.get()).unwrap_or(1),
                            height.map(|h| h.get()).unwrap_or(1),
                        )
                            .into(),
                        false,
                    )
                }
                (_, _) if stateless => (self.stateless_size, true),
                _ => (self.size, true),
            }
        } else {
            match configure.new_size {
                (Some(width), Some(height)) => ((width.get(), height.get()).into(), false),
                _ if stateless => (self.stateless_size, true),
                _ => (self.size, true),
            }
        };

        // Apply configure bounds only when compositor let the user decide what size to pick.
        if constrain {
            let bounds = self.inner_size_bounds(&configure);
            new_size.width = bounds
                .0
                .map(|bound_w| new_size.width.min(bound_w.get()))
                .unwrap_or(new_size.width);
            new_size.height = bounds
                .1
                .map(|bound_h| new_size.height.min(bound_h.get()))
                .unwrap_or(new_size.height);
        }

        // XXX Update the new size right away.
        self.resize(new_size);

        new_size
    }

    /// Compute the bounds for the inner size of the surface.
    fn inner_size_bounds(
        &self,
        configure: &WindowConfigure,
    ) -> (Option<NonZeroU32>, Option<NonZeroU32>) {
        let configure_bounds = match configure.suggested_bounds {
            Some((width, height)) => (NonZeroU32::new(width), NonZeroU32::new(height)),
            None => (None, None),
        };

        if let Some(frame) = self.frame.as_ref() {
            let (width, height) = frame.subtract_borders(
                configure_bounds.0.unwrap_or(NonZeroU32::new(1).unwrap()),
                configure_bounds.1.unwrap_or(NonZeroU32::new(1).unwrap()),
            );
            (
                configure_bounds.0.and(width),
                configure_bounds.1.and(height),
            )
        } else {
            configure_bounds
        }
    }

    #[inline]
    fn is_stateless(configure: &WindowConfigure) -> bool {
        !(configure.is_maximized() || configure.is_fullscreen() || configure.is_tiled())
    }

    /// Start interacting drag resize.
    pub fn drag_resize_window(&self, direction: ResizeDirection) -> Result<(), ExternalError> {
        let wl_shell_surface = self.wl_window.wl_shell_surface();

        // TODO(kchibisov) handle touch serials.
        self.apply_on_poiner(|_, data| {
            let serial = data.latest_button_serial();
            let seat = data.seat();
            wl_shell_surface.resize(seat, serial, direction.into());
        });

        Ok(())
    }

    /// Start the window drag.
    pub fn drag_window(&self) -> Result<(), ExternalError> {
        let wl_shell_surface = self.wl_window.wl_shell_surface();
        // TODO(kchibisov) handle touch serials.
        self.apply_on_poiner(|_, data| {
            let serial = data.latest_button_serial();
            let seat = data.seat();
            wl_shell_surface._move(seat, serial);
        });

        Ok(())
    }

    /// Tells whether the window should be closed.
    #[allow(clippy::too_many_arguments)]
    pub fn frame_click(
        &mut self,
        click: FrameClick,
        pressed: bool,
        seat: &WlSeat,
        serial: u32,
        timestamp: Duration,
        window_id: WindowId,
        updates: &mut Vec<WindowCompositorUpdate>,
    ) -> Option<bool> {
        match self.frame.as_mut()?.on_click(timestamp, click, pressed)? {
            FrameAction::Minimize => {
                // #[cfg(not(feature = "wayland-wl-shell"))]
                // self.xdg_window.set_minimized();
                // For `wl_shell` set_minimized is not available 
            },
            FrameAction::Maximize => {
                self.wl_window.set_maximized();
            },
            FrameAction::UnMaximize => {
                self.wl_window.set_top_level();
            },
            FrameAction::Close => WinitState::queue_close(updates, window_id),
            FrameAction::Move => self.has_pending_move = Some(serial),
            FrameAction::Resize(edge) => {
                let wl_edge = match edge {
                    ResizeEdge::Bottom => Resize::Bottom,
                    ResizeEdge::BottomLeft => Resize::BottomLeft,
                    ResizeEdge::BottomRight => Resize::BottomRight,
                    ResizeEdge::Top => Resize::Top,
                    ResizeEdge::TopLeft => Resize::TopLeft,
                    ResizeEdge::TopRight => Resize::TopRight,
                    ResizeEdge::Left => Resize::Left,
                    ResizeEdge::Right => Resize::Right,
                    ResizeEdge::None => Resize::None,
                    _ => return None
                };
                self.wl_window.resize(seat, serial, wl_edge);
            }
            FrameAction::ShowMenu(_x, _y) => {
                // WlShell has not `show_menu` method
            },
            _ => (),
        };

        Some(false)
    }

    pub fn frame_point_left(&mut self) {
        if let Some(frame) = self.frame.as_mut() {
            frame.click_point_left();
        }
    }

    // Move the point over decorations.
    pub fn frame_point_moved(
        &mut self,
        seat: &WlSeat,
        surface: &WlSurface,
        timestamp: Duration,
        x: f64,
        y: f64,
    ) -> Option<CursorIcon> {
        // Take the serial if we had any, so it doesn't stick around.
        let serial = self.has_pending_move.take();

        if let Some(frame) = self.frame.as_mut() {
            let cursor = frame.click_point_moved(timestamp, &surface.id(), x, y);
            // If we have a cursor change, that means that cursor is over the decorations,
            // so try to apply move.
            if let Some(serial) = cursor.is_some().then_some(serial).flatten() {
                self.wl_window.move_(seat, serial);
                None
            } else {
                cursor
            }
        } else {
            None
        }
    }

    /// Get the stored resizable state.
    #[inline]
    pub fn resizable(&self) -> bool {
        self.resizable
    }

    /// Set the resizable state on the window.
    ///
    /// Returns `true` when the state was applied.
    #[inline]
    pub fn set_resizable(&mut self, resizable: bool) -> bool {
        if self.resizable == resizable {
            return false;
        }

        self.resizable = resizable;
        if resizable {
            // Restore min/max sizes of the window.
            self.reload_min_max_hints();
        } else {
            self.set_min_inner_size(Some(self.size));
            self.set_max_inner_size(Some(self.size));
        }

        // Reload the state on the frame as well.
        if let Some(frame) = self.frame.as_mut() {
            frame.set_resizable(resizable);
        }

        true
    }

    /// Whether the window is focused by any seat.
    #[inline]
    pub fn has_focus(&self) -> bool {
        self.has_focus
    }

    /// Whether the IME is allowed.
    #[inline]
    pub fn ime_allowed(&self) -> bool {
        self.ime_allowed
    }

    /// Get the size of the window.
    #[inline]
    pub fn inner_size(&self) -> LogicalSize<u32> {
        self.size
    }

    /// Whether the window received initial configure event from the compositor.
    #[inline]
    pub fn is_configured(&self) -> bool {
        self.last_configure.is_some()
    }

    #[inline]
    pub fn is_decorated(&mut self) -> bool {
        // There is only server side decorations available for wl_shell.
        true
    }

    /// Get the outer size of the window.
    #[inline]
    pub fn outer_size(&self) -> LogicalSize<u32> {
        self.frame
            .as_ref()
            .map(|frame| frame.add_borders(self.size.width, self.size.height).into())
            .unwrap_or(self.size)
    }

    /// Register pointer on the top-level.
    pub fn pointer_entered(&mut self, added: Weak<ThemedPointer<WinitPointerData>>) {
        self.pointers.push(added);
        self.reload_cursor_style();

        let mode = self.cursor_grab_mode.user_grab_mode;
        let _ = self.set_cursor_grab_inner(mode);
    }

    /// Pointer has left the top-level.
    pub fn pointer_left(&mut self, removed: Weak<ThemedPointer<WinitPointerData>>) {
        let mut new_pointers = Vec::new();
        for pointer in self.pointers.drain(..) {
            if let Some(pointer) = pointer.upgrade() {
                if pointer.pointer() != removed.upgrade().unwrap().pointer() {
                    new_pointers.push(Arc::downgrade(&pointer));
                }
            }
        }

        self.pointers = new_pointers;
    }

    /// Refresh the decorations frame if it's present returning whether the client should redraw.
    pub fn refresh_frame(&mut self) -> bool {
        if let Some(frame) = self.frame.as_mut() {
            if !frame.is_hidden() && frame.is_dirty() {
                return frame.draw();
            }
        }

        false
    }

    /// Reload the cursor style on the given window.
    pub fn reload_cursor_style(&mut self) {
        if self.cursor_visible {
            self.set_cursor(self.cursor_icon);
        } else {
            self.set_cursor_visible(self.cursor_visible);
        }
    }

    /// Reissue the transparency hint to the compositor.
    pub fn reload_transparency_hint(&self) {
        let surface = self.wl_window.wl_surface();

        if self.transparent {
            surface.set_opaque_region(None);
        } else if let Ok(region) = Region::new(&*self.compositor) {
            region.add(0, 0, i32::MAX, i32::MAX);
            surface.set_opaque_region(Some(region.wl_region()));
        } else {
            warn!("Failed to mark window opaque.");
        }
    }

    /// Try to resize the window when the user can do so.
    pub fn request_inner_size(&mut self, inner_size: Size) -> PhysicalSize<u32> {
        if self
            .last_configure
            .as_ref()
            .map(Self::is_stateless)
            .unwrap_or(true)
        {
            self.resize(inner_size.to_logical(self.scale_factor()))
        }

        self.inner_size().to_physical(self.scale_factor())
    }

    /// Resize the window to the new inner size.
    fn resize(&mut self, inner_size: LogicalSize<u32>) {
        self.size = inner_size;

        // Update the stateless size.
        if Some(true) == self.last_configure.as_ref().map(Self::is_stateless) {
            self.stateless_size = inner_size;
        }

        // Update the inner frame.
        let ((_x, _y), _outer_size) = if let Some(frame) = self.frame.as_mut() {
            // Resize only visible frame.
            if !frame.is_hidden() {
                frame.resize(
                    NonZeroU32::new(self.size.width).unwrap(),
                    NonZeroU32::new(self.size.height).unwrap(),
                );
            }

            (
                frame.location(),
                frame.add_borders(self.size.width, self.size.height).into(),
            )
        } else {
            ((0, 0), self.size)
        };

        // Reload the hint.
        self.reload_transparency_hint();

        // Set the window geometry.
        // #[cfg(not(feature = "wayland-wl-shell"))]
        // self.xdg_window.xdg_surface().set_window_geometry(
        //     x,
        //     y,
        //     outer_size.width as i32,
        //     outer_size.height as i32,
        // );

        // Update the target viewport, this is used if and only if fractional scaling is in use.
        if let Some(viewport) = self.viewport.as_ref() {
            // Set inner size without the borders.
            viewport.set_destination(self.size.width as _, self.size.height as _);
        }
    }

    /// Get the scale factor of the window.
    #[inline]
    pub fn scale_factor(&self) -> f64 {
        self.scale_factor
    }

    /// Set the cursor icon.
    ///
    /// Providing `None` will hide the cursor.
    pub fn set_cursor(&mut self, cursor_icon: CursorIcon) {
        self.cursor_icon = cursor_icon;

        if !self.cursor_visible {
            return;
        }

        self.apply_on_poiner(|pointer, _| {
            if pointer.set_cursor(&self.connection, cursor_icon).is_err() {
                warn!("Failed to set cursor to {:?}", cursor_icon);
            }
        })
    }

    /// Set the custom cursor icon.
    // pub(crate) fn set_custom_cursor(&mut self, cursor: RootCustomCursor) {
    //     let cursor = match cursor {
    //         RootCustomCursor {
    //             inner: PlatformCustomCursor::Wayland(cursor),
    //         } => cursor.0,
    //         #[cfg(x11_platform)]
    //         RootCustomCursor {
    //             inner: PlatformCustomCursor::X(_),
    //         } => {
    //             log::error!("passed a X11 cursor to Wayland backend");
    //             return;
    //         }
    //     };

    //     let cursor = {
    //         let mut pool = self.custom_cursor_pool.lock().unwrap();
    //         CustomCursor::new(&mut pool, &cursor)
    //     };

    //     if self.cursor_visible {
    //         self.apply_custom_cursor(&cursor);
    //     }

    //     self.selected_cursor = SelectedCursor::Custom(cursor);
    // }

    // fn apply_custom_cursor(&self, cursor: &CustomCursor) {
    //     self.apply_on_poiner(|pointer, _| {
    //         let surface = pointer.surface();

    //         let scale = surface
    //             .data::<SurfaceData>()
    //             .unwrap()
    //             .surface_data()
    //             .scale_factor();

    //         surface.set_buffer_scale(scale);
    //         surface.attach(Some(cursor.buffer.wl_buffer()), 0, 0);
    //         if surface.version() >= 4 {
    //             surface.damage_buffer(0, 0, cursor.w, cursor.h);
    //         } else {
    //             surface.damage(0, 0, cursor.w / scale, cursor.h / scale);
    //         }
    //         surface.commit();

    //         let serial = pointer
    //             .pointer()
    //             .data::<WinitPointerData>()
    //             .and_then(|data| data.pointer_data().latest_enter_serial())
    //             .unwrap();

    //         pointer.pointer().set_cursor(
    //             serial,
    //             Some(surface),
    //             cursor.hotspot_x / scale,
    //             cursor.hotspot_y / scale,
    //         );
    //     });
    // }

    /// Set maximum inner window size.
    pub fn set_min_inner_size(&mut self, size: Option<LogicalSize<u32>>) {
        // Ensure that the window has the right minimum size.
        let mut size = size.unwrap_or(MIN_WINDOW_SIZE);
        size.width = size.width.max(MIN_WINDOW_SIZE.width);
        size.height = size.height.max(MIN_WINDOW_SIZE.height);

        // Add the borders.
        let size = self
            .frame
            .as_ref()
            .map(|frame| frame.add_borders(size.width, size.height).into())
            .unwrap_or(size);

        self.min_inner_size = size;
    }

    /// Set maximum inner window size.
    pub fn set_max_inner_size(&mut self, size: Option<LogicalSize<u32>>) {
        let size = size.map(|size| {
            self.frame
                .as_ref()
                .map(|frame| frame.add_borders(size.width, size.height).into())
                .unwrap_or(size)
        });

        self.max_inner_size = size;
    }

    /// Set the CSD theme.
    pub fn set_theme(&mut self, theme: Option<Theme>) {
        self.theme = theme;
        #[cfg(feature = "sctk-adwaita")]
        if let Some(frame) = self.frame.as_mut() {
            frame.set_config(into_sctk_adwaita_config(theme))
        }
    }

    /// The current theme for CSD decorations.
    #[inline]
    pub fn theme(&self) -> Option<Theme> {
        self.theme
    }

    /// Set the cursor grabbing state on the top-level.
    pub fn set_cursor_grab(&mut self, mode: CursorGrabMode) -> Result<(), ExternalError> {
        // Replace the user grabbing mode.
        self.cursor_grab_mode.user_grab_mode = mode;
        self.set_cursor_grab_inner(mode)
    }

    /// Reload the hints for minimum and maximum sizes.
    pub fn reload_min_max_hints(&mut self) {
        self.set_min_inner_size(Some(self.min_inner_size));
        self.set_max_inner_size(self.max_inner_size);
    }

    /// Set the grabbing state on the surface.
    fn set_cursor_grab_inner(&mut self, mode: CursorGrabMode) -> Result<(), ExternalError> {
        let pointer_constraints = match self.pointer_constraints.as_ref() {
            Some(pointer_constraints) => pointer_constraints,
            None if mode == CursorGrabMode::None => return Ok(()),
            None => return Err(ExternalError::NotSupported(NotSupportedError::new())),
        };

        // Replace the current mode.
        let old_mode = std::mem::replace(&mut self.cursor_grab_mode.current_grab_mode, mode);

        match old_mode {
            CursorGrabMode::None => (),
            CursorGrabMode::Confined => self.apply_on_poiner(|_, data| {
                data.unconfine_pointer();
            }),
            CursorGrabMode::Locked => {
                self.apply_on_poiner(|_, data| data.unlock_pointer());
            }
        }

        let surface = self.wl_window.wl_surface();

        match mode {
            CursorGrabMode::Locked => self.apply_on_poiner(|pointer, data| {
                let pointer = pointer.pointer();
                data.lock_pointer(pointer_constraints, surface, pointer, &self.queue_handle)
            }),
            CursorGrabMode::Confined => self.apply_on_poiner(|pointer, data| {
                let pointer = pointer.pointer();
                data.confine_pointer(pointer_constraints, surface, pointer, &self.queue_handle)
            }),
            CursorGrabMode::None => {
                // Current lock/confine was already removed.
            }
        }

        Ok(())
    }

    pub fn show_window_menu(&self, _position: LogicalPosition<u32>) {
        // TODO(kchibisov) handle touch serials.
        // #[cfg(not(feature = "wayland-wl-shell"))]
        // self.apply_on_poiner(|_, data| {
        //     let serial = data.latest_button_serial();
        //     let seat = data.seat();
        //     // self.wl_window.show_window_menu(seat, serial, position.into());
        // });
    }

    /// Set the position of the cursor.
    pub fn set_cursor_position(&self, position: LogicalPosition<f64>) -> Result<(), ExternalError> {
        if self.pointer_constraints.is_none() {
            return Err(ExternalError::NotSupported(NotSupportedError::new()));
        }

        // Positon can be set only for locked cursor.
        if self.cursor_grab_mode.current_grab_mode != CursorGrabMode::Locked {
            return Err(ExternalError::Os(os_error!(
                crate::platform_impl::OsError::Misc(
                    "cursor position can be set only for locked cursor."
                )
            )));
        }

        self.apply_on_poiner(|_, data| {
            data.set_locked_cursor_position(position.x, position.y);
        });

        Ok(())
    }

    /// Set the visibility state of the cursor.
    pub fn set_cursor_visible(&mut self, cursor_visible: bool) {
        self.cursor_visible = cursor_visible;

        if self.cursor_visible {
            self.set_cursor(self.cursor_icon);
        } else {
            for pointer in self.pointers.iter().filter_map(|pointer| pointer.upgrade()) {
                let latest_enter_serial = pointer.pointer().winit_data().latest_enter_serial();

                pointer
                    .pointer()
                    .set_cursor(latest_enter_serial, None, 0, 0);
            }
        }
    }

    /// Whether show or hide client side decorations.
    #[inline]
    pub fn set_decorate(&mut self, decorate: bool) {
        if decorate == self.decorate {
            return;
        }

        self.decorate = decorate;
        
        // #[cfg(not(feature = "wayland-wl-shell"))]
        // match self
        //     .last_configure
        //     .as_ref()
        //     .map(|configure| configure.decoration_mode)
        // {
        //     Some(DecorationMode::Server) if !self.decorate => {
        //         // To disable decorations we should request client and hide the frame.
        //         self.xdg_window.request_decoration_mode(Some(DecorationMode::Client));
        //     }
        //     _ if self.decorate => {
        //         self.xdg_window.request_decoration_mode(Some(DecorationMode::Server));
        //     },
        //     _ => (),
        // }

        if let Some(frame) = self.frame.as_mut() {
            frame.set_hidden(!decorate);
            // Force the resize.
            self.resize(self.size);
        }
    }

    /// Mark that the window has focus.
    ///
    /// Should be used from routine that sends focused event.
    #[inline]
    pub fn set_has_focus(&mut self, has_focus: bool) {
        self.has_focus = has_focus;
    }

    /// Returns `true` if the requested state was applied.
    pub fn set_ime_allowed(&mut self, allowed: bool) -> bool {
        self.ime_allowed = allowed;

        let mut applied = false;
        for text_input in &self.text_inputs {
            applied = true;
            if allowed {
                text_input.enable();
                text_input.set_content_type_by_purpose(self.ime_purpose);
            } else {
                text_input.disable();
            }
            text_input.commit();
        }

        applied
    }

    /// Set the IME position.
    pub fn set_ime_cursor_area(&self, position: LogicalPosition<u32>, size: LogicalSize<u32>) {
        // FIXME: This won't fly unless user will have a way to request IME window per seat, since
        // the ime windows will be overlapping, but winit doesn't expose API to specify for
        // which seat we're setting IME position.
        let (x, y) = (position.x as i32, position.y as i32);
        let (width, height) = (size.width as i32, size.height as i32);
        for text_input in self.text_inputs.iter() {
            text_input.set_cursor_rectangle(x, y, width, height);
            text_input.commit();
        }
    }

    /// Set the IME purpose.
    pub fn set_ime_purpose(&mut self, purpose: ImePurpose) {
        self.ime_purpose = purpose;

        for text_input in &self.text_inputs {
            text_input.set_content_type_by_purpose(purpose);
            text_input.commit();
        }
    }

    /// Get the IME purpose.
    pub fn ime_purpose(&self) -> ImePurpose {
        self.ime_purpose
    }

    /// Set the scale factor for the given window.
    #[inline]
    pub fn set_scale_factor(&mut self, scale_factor: f64) {
        self.scale_factor = scale_factor;

        // NOTE: When fractional scaling is not used update the buffer scale.
        if self.fractional_scale.is_none() {
            let _ = self.wl_window.set_buffer_scale(self.scale_factor as _);
        }

        if let Some(frame) = self.frame.as_mut() {
            frame.set_scaling_factor(scale_factor);
        }
    }

    /// Make window background blurred
    #[inline]
    pub fn set_blur(&mut self, blurred: bool) {
        let wl_surface = self.wl_window.wl_surface();

        if blurred && self.blur.is_none() {
            if let Some(blur_manager) = self.blur_manager.as_ref() {
                let blur = blur_manager.blur(wl_surface, &self.queue_handle);
                blur.commit();
                self.blur = Some(blur);
            } else {
                info!("Blur manager unavailable, unable to change blur")
            }
        } else if !blurred && self.blur.is_some() {
            self.blur_manager
                .as_ref()
                .unwrap()
                .unset(wl_surface);
            self.blur.take().unwrap().release();
        }
    }

    /// Set the window title to a new value.
    ///
    /// This will autmatically truncate the title to something meaningfull.
    pub fn set_title(&mut self, mut title: String) {
        // Truncate the title to at most 1024 bytes, so that it does not blow up the protocol
        // messages
        if title.len() > 1024 {
            let mut new_len = 1024;
            while !title.is_char_boundary(new_len) {
                new_len -= 1;
            }
            title.truncate(new_len);
        }

        // Update the CSD title.
        if let Some(frame) = self.frame.as_mut() {
            frame.set_title(&title);
        }

        self.wl_window.set_title(&title);
        self.title = title;
    }

    /// Mark the window as transparent.
    #[inline]
    pub fn set_transparent(&mut self, transparent: bool) {
        self.transparent = transparent;
        self.reload_transparency_hint();
    }

    /// Register text input on the top-level.
    #[inline]
    pub fn text_input_entered(&mut self, text_input: &ZwpTextInputV3) {
        if !self.text_inputs.iter().any(|t| t == text_input) {
            self.text_inputs.push(text_input.clone());
        }
    }

    /// The text input left the top-level.
    #[inline]
    pub fn text_input_left(&mut self, text_input: &ZwpTextInputV3) {
        if let Some(position) = self.text_inputs.iter().position(|t| t == text_input) {
            self.text_inputs.remove(position);
        }
    }

    /// Get the cached title.
    #[inline]
    pub fn title(&self) -> &str {
        &self.title
    }
}

impl Drop for WindowState {
    fn drop(&mut self) {
        if let Some(blur) = self.blur.take() {
            blur.release();
        }

        if let Some(fs) = self.fractional_scale.take() {
            fs.destroy();
        }

        if let Some(viewport) = self.viewport.take() {
            viewport.destroy();
        }

        // NOTE: the wl_surface used by the window is being cleaned up when
        // dropping SCTK `Window`.
    }
}

/// The state of the cursor grabs.
#[derive(Clone, Copy)]
struct GrabState {
    /// The grab mode requested by the user.
    user_grab_mode: CursorGrabMode,

    /// The current grab mode.
    current_grab_mode: CursorGrabMode,
}

impl GrabState {
    fn new() -> Self {
        Self {
            user_grab_mode: CursorGrabMode::None,
            current_grab_mode: CursorGrabMode::None,
        }
    }
}

/// The state of the frame callback.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameCallbackState {
    /// No frame callback was requsted.
    #[default]
    None,
    /// The frame callback was requested, but not yet arrived, the redraw events are throttled.
    Requested,
    /// The callback was marked as done, and user could receive redraw requested
    Received,
}

// NOTE: Rust doesn't allow `From<Option<Theme>>`.
#[cfg(feature = "sctk-adwaita")]
fn into_sctk_adwaita_config(theme: Option<Theme>) -> sctk_adwaita::FrameConfig {
    match theme {
        Some(Theme::Light) => sctk_adwaita::FrameConfig::light(),
        Some(Theme::Dark) => sctk_adwaita::FrameConfig::dark(),
        None => sctk_adwaita::FrameConfig::auto(),
    }
}
